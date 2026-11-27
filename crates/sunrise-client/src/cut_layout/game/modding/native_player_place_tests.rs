//! `player.place_block` against real LAN guests, through the public player handle, with a
//! literal test for each refusal.

use mod_api::{BlockFace, EntityRef, PlayerWorldRefusal as Refusal, WorldEpoch};
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_gameplay_common::game::{
    native_block_entity_read::NativeBlockEntityReadGameExt1 as _,
    transactions::TransactionsGameExt as _,
};

/// The API's value for a block state of the world.
fn api_state<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &G,
    name: &str,
) -> mod_api::BlockState {
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap()
        .try_into_api()
        .unwrap()
}

fn loaded(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    // The write is `level_set_block`'s, which an authoritative server only takes.
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
    );
}

/// A bed written for a player gets its head (`setPlacedBy`).
#[test]
#[ignore = "daily"]
fn a_block_placed_as_a_player_runs_set_placed_by() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let bed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:red_bed")
            .unwrap();
        let api_bed = api_state(game, "minecraft:red_bed");
        assert_eq!(
            b.place_block(game, level, [2, 70, 2], api_bed, BlockFace::Up, true),
            Ok(())
        );
        assert_eq!(game.world.block(2, 70, 2), bed);
        let head = game.world.block(2, 70, 1);
        let facts = game.res.facts();
        assert_eq!(
            facts.get(head).name,
            facts.get(bed).name,
            "the head was placed"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_plain_write_before_placement_preserves_the_staged_chest_transfer() {
    use mod_api::{BlockUpdateFlags, ITEMS, ItemResource};
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_items::items::ItemStack;

    if run_fresh_cases(
        "cut_layout::game::native_player_place_tests::a_plain_write_before_placement_preserves_the_staged_chest_transfer",
        "SUNRISE_TEST_STAGED_CHEST_TRANSFER",
        &["server"],
    ) {
        return;
    }
    const SOURCE: [i32; 3] = [5, 70, 2];
    const DEST: [i32; 3] = [2, 70, 2];
    with_guests(|game| {
        loaded(game);
        let level = game.level(game.dim.id()).unwrap();
        let player = game.player(12).unwrap();
        let source = api_state(game, "minecraft:barrel");
        assert_eq!(
            level.set_block(game, SOURCE, source, BlockUpdateFlags::NONE),
            Ok(true)
        );
        let mut stored = ItemStack::new("minecraft:iron_ingot", 17);
        game.world
            .block_entities
            .get_mut(&(5, 70, 2))
            .unwrap()
            .slots_mut()[0] = Some(stored.clone());
        let resource = ItemResource::of(&stored.clone().try_into_api().unwrap());
        let from = level.capability(game, &ITEMS, SOURCE, None).unwrap();
        let registry = game.pack.assets.registry;
        let chest = registry.default_state("minecraft:chest").unwrap();
        let north = sunrise_gameplay_common::game::native_hooks::with_property(
            registry, chest, "facing", "north",
        )
        .unwrap()
        .try_into_api()
        .unwrap();
        let south = sunrise_gameplay_common::game::native_hooks::with_property(
            registry, chest, "facing", "south",
        )
        .unwrap()
        .try_into_api()
        .unwrap();

        let staged = game
            .storage_transaction(|game, tx| {
                assert_eq!(
                    level.set_block(game, DEST, north, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                let staged = game.block_entity(level, DEST).unwrap();
                player.place_block(game, level, DEST, south, BlockFace::Up, false)?;
                assert_eq!(game.block_entity(level, DEST), Some(staged));
                let to = level.capability(game, &ITEMS, DEST, None).unwrap();
                assert_eq!(from.extract_at(game, tx, 0, &resource, 12), Ok(12));
                assert_eq!(to.insert_at(game, tx, 0, &resource, 12), Ok(12));
                Ok::<_, Refusal>(staged)
            })
            .unwrap();

        assert_eq!(level.block_state(game, DEST), Some(south));
        assert_eq!(
            game.world.block_entities.get(&(2, 70, 2)).unwrap().slots()[0],
            Some(ItemStack::new("minecraft:iron_ingot", 12))
        );
        stored.count = 5;
        assert_eq!(
            game.world.block_entities.get(&(5, 70, 2)).unwrap().slots()[0],
            Some(stored)
        );
        assert_eq!(
            game.block_entity(level, DEST),
            Some(staged),
            "commit keeps the generation named by the staged handle"
        );
    });
}

#[test]
#[ignore = "daily"]
fn chosen_placement_attributes_the_player_and_rolls_back_on_refusal() {
    use mod_api::{
        BlockEdit, BlockEntityRef, BlockPart, BlockRun, BlockState, BlockUpdateFlags, BlocksEvent,
        EntityPlaceEvent, Flow, Hooks, ITEMS, ItemResource, LevelTicksHost, LivingRef, ModEntry,
        Mods, Placement, Registrar, RegistrationContext, ServerHost, UpdateShapeEvent,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_core::{nbt::Tag, position::BlockPos};
    use sunrise_gameplay::game::native_entity_refs::tests::{fixture_root, run_fresh_cases};
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests_at;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::{BlockEntity, ModBlockEntity, ModContainer};

    const FLAG: &str = "SUNRISE_TEST_PLAYER_PLACE";
    const AT: [i32; 3] = [2, 70, 2];
    const HEAD: [i32; 3] = [2, 70, 1];
    const AGAINST: [i32; 3] = [1, 70, 2];
    const WRITE: [i32; 3] = [4, 70, 2];
    const OUTER: [i32; 3] = [5, 70, 2];
    const NAME: &str = "The placed chest";
    const ADJACENT: [i32; 3] = [2, 71, 2];
    const ADJACENT_BLOCK: &str = "placement_probe:adjacent";
    if run_fresh_cases(
        "cut_layout::game::native_player_place_tests::chosen_placement_attributes_the_player_and_rolls_back_on_refusal",
        FLAG,
        &[
            "chest-accept",
            "chest-veto",
            "chest-compatible-accept",
            "chest-compatible-veto",
            "chest-compatible-bulk-accept",
            "chest-compatible-bulk-veto",
            "chest-compatible-retire",
            "chest-replace",
            "chest-round-trip",
            "chest-skip-removal",
            "chest-transfer",
            "bed-head",
            "bed-veto",
            "bed-stale",
            "bed-against",
            "bed-listener-panic",
            "bed-death",
        ],
    ) {
        return;
    }
    type PlacementSeen = (
        Option<EntityRef>,
        BlockPos,
        BlockState,
        BlockState,
        Placement,
    );
    struct Probe {
        case: String,
        seen: Arc<Mutex<Vec<PlacementSeen>>>,
        entity: Arc<Mutex<Option<BlockEntityRef>>>,
    }
    struct AdjacentShape;

    #[mod_api::sunrise_part]
    impl BlockPart for AdjacentShape {
        fn update_shape(
            &self,
            level: &mut dyn LevelTicksHost,
            event: &mut UpdateShapeEvent,
        ) -> Option<BlockState> {
            let origin = BlockPos::new(AT[0], AT[1], AT[2]);
            if event.neighbour_pos == origin
                && Some(event.neighbour) == level.block_state_id("minecraft:barrel")
            {
                return level.block_state_id("minecraft:granite");
            }
            None
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            if self.case == "chest-skip-removal" {
                event
                    .register(mod_api::Block::new(ADJACENT_BLOCK, "minecraft:stone"))
                    .unwrap();
            }
        }

        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            if self.case == "chest-skip-removal" {
                let block = registrar.block_key(ADJACENT_BLOCK);
                registrar.block_part(block, AdjacentShape);
            }
        }

        fn entity_place(&self, host: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            let level = host.level("minecraft:overworld").unwrap();
            assert_eq!(level.block_state(host, AT), Some(event.state));
            self.seen.lock().unwrap().push((
                event.placer,
                event.pos,
                event.state,
                event.placed_against,
                event.placement.clone(),
            ));
            if self.case.starts_with("chest-") {
                let entity = host.block_entity(level, AT).expect("tentative chest");
                assert!(entity.data(host).is_some());
                assert_eq!(entity.set_name(host, Some(NAME)), Ok(true));
                *self.entity.lock().unwrap() = Some(entity);
            } else {
                assert!(host.block_entity(level, AT).is_none());
                let Placement::Multi(blocks) = &event.placement else {
                    panic!("one event for both halves")
                };
                assert_eq!(level.block_state(host, HEAD), Some(blocks[1].state));
                let write = match self.case.as_str() {
                    "bed-stale" => Some(HEAD),
                    "bed-against" => Some(AGAINST),
                    "bed-veto" | "bed-listener-panic" => Some(WRITE),
                    _ => None,
                };
                if let Some(write) = write {
                    let edit = if self.case == "bed-against" {
                        level.block_state(host, WRITE).unwrap()
                    } else {
                        event.state
                    };
                    assert_eq!(
                        level.set_block(host, write, edit, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                }
                if self.case == "bed-listener-panic" {
                    panic!("placement listener unwound");
                }
            }
            if self.case.ends_with("-veto") {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    struct AfterPanic {
        earlier: Arc<Mutex<Vec<PlacementSeen>>>,
        read: Arc<Mutex<Option<BlockState>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for AfterPanic {
        fn entity_place(&self, host: &mut dyn ServerHost, _: &mut EntityPlaceEvent) -> Flow {
            assert_eq!(
                self.earlier.lock().unwrap().len(),
                1,
                "runs after the failed listener"
            );
            let level = host.level("minecraft:overworld").unwrap();
            *self.read.lock().unwrap() = level.block_state(host, WRITE);
            Flow::Continue
        }
    }
    let case = std::env::var(FLAG).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let entity = Arc::new(Mutex::new(None));
    let later_read = Arc::new(Mutex::new(None));
    let skip_removal = case == "chest-skip-removal";
    let mut entries = vec![ModEntry::new(
        "placement_probe",
        Box::new(Probe {
            case: case.clone(),
            seen: seen.clone(),
            entity: entity.clone(),
        }),
    )];
    if case == "bed-listener-panic" {
        entries.push(ModEntry::new(
            "placement_probe_tail",
            Box::new(AfterPanic {
                earlier: seen.clone(),
                read: later_read.clone(),
            }),
        ));
    }
    let mods: Mods = entries.into();
    with_mods(mods.clone(), || {
        let base = fixture_root();
        let prepared = skip_removal.then(|| {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap()
        });
        let root = prepared
            .as_ref()
            .map_or(base, |root| root.prepared().to_path_buf());
        with_guests_at(root, |game| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            loaded(game);
            let place_as = game.player(13).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let foot_id = game
                .pack
                .assets
                .registry
                .default_state("minecraft:red_bed")
                .unwrap();
            let foot: BlockState = foot_id.try_into_api().unwrap();
            let head_id = game
                .with_props(foot_id, &[("part", "head"), ("facing", "north")])
                .unwrap();
            let head: BlockState = head_id.try_into_api().unwrap();
            let air: BlockState = game.world.air.try_into_api().unwrap();
            let stone = api_state(game, "minecraft:stone");
            let chest = api_state(game, "minecraft:chest");
            let barrel = api_state(game, "minecraft:barrel");
            let is_bed = case.starts_with("bed-");
            let compatible = case.starts_with("chest-compatible-");
            let facing_chest = |direction| {
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                game.with_props(state, &[("facing", direction)])
                    .unwrap()
                    .try_into_api()
                    .unwrap()
            };
            let north = facing_chest("north");
            let south = facing_chest("south");
            let (before, chosen) = match case.as_str() {
                _ if is_bed => (chest, foot),
                _ if compatible => (air, south),
                "chest-replace" => (barrel, chest),
                "chest-round-trip" => (chest, chest),
                "chest-skip-removal" => (chest, barrel),
                _ => (air, chest),
            };
            if before != air {
                assert_eq!(
                    level.set_block(game, AT, before, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            assert_eq!(
                level.set_block(game, AGAINST, stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
            let stack = ItemStack::new("minecraft:dirt", 5);
            game.as_guest(13, |game| {
                game.common.inventory[game.common.selected] = Some(stack.clone())
            })
            .unwrap();
            if is_bed {
                game.world.block_entities.insert(
                    (2, 70, 2),
                    BlockEntity::Chest(vec![Some(stack.clone()); 27]),
                );
            }
            let original_entity = game.block_entity(level, AT);
            if skip_removal {
                let adjacent = game
                    .pack
                    .assets
                    .registry
                    .default_state(ADJACENT_BLOCK)
                    .unwrap();
                assert_eq!(
                    level.set_block(
                        game,
                        ADJACENT,
                        adjacent.try_into_api().unwrap(),
                        BlockUpdateFlags::NONE
                    ),
                    Ok(true)
                );
                game.world.block_entities.set_mod_containers(
                    [(
                        adjacent.0,
                        Arc::new(ModContainer {
                            slots: 1,
                            ..Default::default()
                        }),
                    )]
                    .into_iter()
                    .collect(),
                );
                let mut container =
                    ModBlockEntity::new("placement_probe:container".into(), Default::default());
                container
                    .items
                    .push(Some(ItemStack::new("minecraft:gold_ingot", 11)));
                game.world.block_entities.insert(
                    (ADJACENT[0], ADJACENT[1], ADJACENT[2]),
                    BlockEntity::Mod(Box::new(container)),
                );
            }
            game.world.block_log = Some(Vec::new());
            game.world.bulk_log.clear();
            let place = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.as_guest(12, |game| {
                    let face = if case == "bed-head" {
                        BlockFace::South
                    } else {
                        BlockFace::East
                    };
                    place_as.place_block(game, level, AT, chosen, face, skip_removal)
                })
                .unwrap()
            };
            let result = match case.as_str() {
                _ if compatible => game.storage_transaction(|game, _| {
                    if case.starts_with("chest-compatible-bulk-") {
                        let [x, y, z] = AT;
                        let edit = BlockEdit {
                            palette: vec![north],
                            filters: vec![],
                            runs: vec![BlockRun {
                                x,
                                y,
                                z,
                                block: 0,
                                height: 1,
                                surface: None,
                                only: None,
                            }],
                        };
                        assert_eq!(level.edit_blocks(game, &edit), Ok(1));
                    } else {
                        assert_eq!(
                            level.set_block(game, AT, north, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                    }
                    let original = game.block_entity(level, AT).unwrap();
                    let result = place(game);
                    assert_eq!(*entity.lock().unwrap(), Some(original));
                    assert_eq!(game.block_entity(level, AT), Some(original));
                    assert!(game.world.block_log.as_ref().unwrap().is_empty());
                    assert!(game.world.bulk_log.is_empty());
                    result?;
                    if case == "chest-compatible-retire" {
                        use sunrise_level::{
                            chunk::PendingContainerLoot, world::BlockEntityExtras,
                        };
                        let pos = (AT[0], AT[1], AT[2]);
                        assert_eq!(
                            game.world.block_names.get(&pos),
                            Some(&Tag::String(NAME.into()))
                        );
                        let extras = BlockEntityExtras {
                            source: Some(Tag::Compound(Default::default())),
                            pending_loot: Some(PendingContainerLoot {
                                table: Tag::String("minecraft:empty".into()),
                                seed: None,
                            }),
                        };
                        game.world.set_block_entity_extras(pos, extras.clone());
                        let replace = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                            assert_eq!(
                                level.set_block(game, AT, barrel, BlockUpdateFlags::NONE),
                                Ok(true)
                            );
                            let replacement = game.block_entity(level, AT).unwrap();
                            assert_ne!(replacement, original);
                            assert_eq!(
                                game.world.block_entity_extras(pos),
                                BlockEntityExtras::default()
                            );
                            assert!(!game.world.block_names.contains_key(&pos));
                        };
                        game.storage_scope(|game, _| {
                            replace(game);
                            ((), false)
                        });
                        assert_eq!(game.block_entity(level, AT), Some(original));
                        assert_eq!(
                            game.world.block_names.get(&pos),
                            Some(&Tag::String(NAME.into()))
                        );
                        assert_eq!(game.world.block_entity_extras(pos), extras);
                        replace(game);
                    }
                    Ok(())
                }),
                "chest-round-trip" | "chest-skip-removal" => {
                    game.world
                        .block_entities
                        .get_mut(&(2, 70, 2))
                        .unwrap()
                        .slots_mut()
                        .fill(Some(stack.clone()));
                    let from = level.capability(game, &ITEMS, AT, None).unwrap();
                    let dirt = ItemResource::of(&stack.clone().try_into_api().unwrap());
                    let new_stack = ItemStack::new("minecraft:stone", 7);
                    let replacement = ItemResource::of(&new_stack.try_into_api().unwrap());
                    let removal_flags = if skip_removal {
                        BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS
                    } else {
                        BlockUpdateFlags::NONE
                    };
                    game.storage_transaction(|game, tx| {
                        assert_eq!(from.extract_at(game, tx, 0, &dirt, 2), Ok(2));
                        assert_eq!(level.set_block(game, AT, stone, removal_flags), Ok(true));
                        place(game)?;
                        let to = level.capability(game, &ITEMS, AT, None).unwrap();
                        assert_eq!(to.insert_at(game, tx, 0, &replacement, 7), Ok(7));
                        assert!(game.dropped.is_empty());
                        Ok(())
                    })
                }
                "chest-transfer" => {
                    assert_eq!(
                        level.set_block(game, OUTER, barrel, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    game.world
                        .block_entities
                        .get_mut(&(5, 70, 2))
                        .unwrap()
                        .slots_mut()[0] = Some(ItemStack::new("minecraft:dirt", 17));
                    let from = level.capability(game, &ITEMS, OUTER, None).unwrap();
                    let dirt = ItemResource::of(&stack.clone().try_into_api().unwrap());
                    game.storage_transaction(|game, tx| {
                        place(game)?;
                        let to = level.capability(game, &ITEMS, AT, None).unwrap();
                        assert_eq!(from.extract_at(game, tx, 0, &dirt, 12), Ok(12));
                        assert_eq!(to.insert_at(game, tx, 0, &dirt, 12), Ok(12));
                        assert_eq!(
                            level.set_block(game, AT, stone, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                        assert!(
                            game.dropped.is_empty(),
                            "removal is not published before commit"
                        );
                        Ok(())
                    })
                }
                "bed-listener-panic" => game.storage_transaction(|game, outer| {
                    assert_eq!(
                        level.set_block(game, OUTER, stone, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    place(game)?;
                    assert!(game.transaction_matches(Some(outer)));
                    assert_eq!(level.block_state(game, OUTER), Some(stone));
                    assert_eq!(*later_read.lock().unwrap(), Some(air));
                    assert_eq!(level.block_state(game, AT), Some(foot));
                    assert_eq!(level.block_state(game, WRITE), Some(air));
                    Ok(())
                }),
                "bed-death" => game.storage_transaction(|game, _| {
                    place(game)?;
                    assert_eq!(level.block_state(game, HEAD), Some(head));
                    let living = LivingRef::new(place_as.entity());
                    assert!(living.set_health(game, 0.0));
                    assert_eq!(living.health(game), Some(0.0));
                    Ok(())
                }),
                _ => place(game),
            };
            let seen = seen.lock().unwrap();
            let [(placer, pos, state, against, footprint)] = seen.as_slice() else {
                panic!("one placement event")
            };
            assert_eq!(
                (*placer, *pos, *state, *against),
                (
                    Some(place_as.entity()),
                    BlockPos::new(2, 70, 2),
                    chosen,
                    if case == "bed-head" { head } else { stone }
                )
            );
            if is_bed {
                let Placement::Multi(blocks) = footprint else {
                    panic!("both bed halves")
                };
                assert_eq!(
                    blocks
                        .iter()
                        .map(|cell| (cell.pos, cell.state))
                        .collect::<Vec<_>>(),
                    [
                        (BlockPos::new(2, 70, 2), foot),
                        (BlockPos::new(2, 70, 1), head)
                    ]
                );
            } else {
                assert!(matches!(footprint, Placement::Single));
            }
            if case != "bed-death" {
                game.as_guest(13, |game| assert_eq!(game.held(), Some(&stack)))
                    .unwrap();
            }
            match case.as_str() {
                "bed-death" | "bed-against" | "bed-head" | "bed-listener-panic" => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(level.block_state(game, AT), Some(foot));
                    assert_eq!(level.block_state(game, HEAD), Some(head));
                    assert_eq!(
                        level.block_state(game, AGAINST),
                        Some(if case == "bed-against" { air } else { stone })
                    );
                    if case == "bed-listener-panic" {
                        assert_eq!(level.block_state(game, OUTER), Some(stone));
                        assert_eq!(level.block_state(game, WRITE), Some(air));
                        assert!(game.transaction_matches(None));
                    }
                }
                "chest-transfer" => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(level.block_state(game, AT), Some(stone));
                    assert!(game.block_entity(level, AT).is_none());
                    let source = game.world.block_entities.get(&(5, 70, 2)).unwrap();
                    assert_eq!(source.slots()[0].as_ref(), Some(&stack));
                    assert_eq!(
                        game.dropped
                            .iter()
                            .filter(|item| &*item.stack.item == "minecraft:dirt")
                            .map(|item| item.stack.count)
                            .sum::<u32>(),
                        12
                    );
                }
                "chest-round-trip" | "chest-skip-removal" => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(level.block_state(game, AT), Some(chosen));
                    let final_entity = game.block_entity(level, AT).unwrap();
                    assert_eq!(Some(final_entity), *entity.lock().unwrap());
                    assert_ne!(Some(final_entity), original_entity);
                    let slots = game.world.block_entities.get(&(2, 70, 2)).unwrap().slots();
                    assert_eq!(slots[0], Some(ItemStack::new("minecraft:stone", 7)));
                    assert!(slots[1..].iter().all(Option::is_none));
                    let (dropped_item, count) = if skip_removal {
                        assert_eq!(
                            level.block_state(game, ADJACENT),
                            Some(api_state(game, "minecraft:granite"))
                        );
                        assert!(game.block_entity(level, ADJACENT).is_none());
                        // The original chest's dirt stays suppressed, but the barrel's own
                        // shape updates must spill the adjacent container's gold.
                        ("minecraft:gold_ingot", 11)
                    } else {
                        ("minecraft:dirt", 133)
                    };
                    assert!(
                        game.dropped
                            .iter()
                            .all(|item| &*item.stack.item == dropped_item)
                    );
                    assert_eq!(
                        game.dropped
                            .iter()
                            .map(|item| item.stack.count)
                            .sum::<u32>(),
                        count
                    );
                }
                "chest-compatible-retire" => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(level.block_state(game, AT), Some(barrel));
                    let replacement = game.block_entity(level, AT).unwrap();
                    assert_ne!(Some(replacement), *entity.lock().unwrap());
                    assert!(!game.world.block_names.contains_key(&(2, 70, 2)));
                    assert_eq!(
                        game.world.block_entity_extras((2, 70, 2)),
                        Default::default()
                    );
                }
                "chest-accept"
                | "chest-replace"
                | "chest-compatible-accept"
                | "chest-compatible-bulk-accept" => {
                    assert_eq!(result, Ok(()));
                    assert_eq!(level.block_state(game, AT), Some(chosen));
                    assert_eq!(game.block_entity(level, AT), *entity.lock().unwrap());
                    assert_eq!(
                        game.world.block_names.get(&(2, 70, 2)),
                        Some(&Tag::String(NAME.into()))
                    );
                    if compatible {
                        assert_eq!(
                            game.world.bulk_log.contains(&(0, 0)),
                            case == "chest-compatible-bulk-accept"
                        );
                        let placement_updates = usize::from(game.test_block_ticking(0, 0));
                        let at = (AT[0], AT[1], AT[2]);
                        let updates = game.world.block_log.as_ref().unwrap();
                        assert_eq!(
                            updates.iter().filter(|&&pos| pos == at).count(),
                            placement_updates + 1,
                            "the accepted data update is sent in addition to any placement update"
                        );
                    }
                }
                "chest-veto"
                | "chest-compatible-veto"
                | "chest-compatible-bulk-veto"
                | "bed-veto"
                | "bed-stale" => {
                    assert_eq!(
                        result,
                        if case == "bed-stale" {
                            Err(Refusal::PlacementChanged)
                        } else {
                            Err(Refusal::Canceled)
                        }
                    );
                    assert_eq!(level.block_state(game, AT), Some(before));
                    assert_eq!(level.block_state(game, AGAINST), Some(stone));
                    assert_eq!(game.block_entity(level, AT), original_entity);
                    assert!(!game.world.block_names.contains_key(&(2, 70, 2)));
                    if is_bed {
                        assert!(matches!(game.world.block_entities.get(&(2, 70, 2)),
                            Some(BlockEntity::Chest(slots)) if slots[0].as_ref() == Some(&stack)));
                    }
                    assert_eq!(level.block_state(game, WRITE), Some(air));
                    assert_eq!(level.block_state(game, HEAD), Some(air));
                    assert!(game.world.block_log.as_ref().unwrap().is_empty());
                    assert!(game.world.bulk_log.is_empty());
                }
                _ => unreachable!(),
            }
        })
    });
}

#[test]
#[ignore = "daily"]
fn set_placed_by_writes_are_part_of_player_placement() {
    use mod_api::{
        BlockPart, BlockUpdateFlags, EntityPlaceEvent, Flow, Hooks, ModEntry, Placement, Registrar,
        RegistrationContext, ServerHost, SetPlacedByEvent,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

    const TEST: &str = "cut_layout::game::native_player_place_tests::set_placed_by_writes_are_part_of_player_placement";
    const FLAG: &str = "SUNRISE_TEST_PLAYER_PLACE_WRITES";
    const ORIGIN: [i32; 3] = [5, 72, 4];
    const ABOVE: [i32; 3] = [5, 73, 4];
    if run_fresh_cases(
        TEST,
        FLAG,
        &[
            "part-accept",
            "part-veto",
            "wither-accept",
            "wither-veto",
            "destroy-no-listener",
            "destroy-collect",
            "destroy-veto",
        ],
    ) {
        return;
    }

    #[derive(Clone)]
    struct Probe {
        veto: bool,
        destroy: bool,
        collect: bool,
        placed_by: Arc<AtomicUsize>,
        writes: Arc<Mutex<Vec<[i32; 3]>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let wool = r.block_key("minecraft:white_wool");
            r.block_part(wool, self.clone());
        }
    }
    struct Listen(Probe);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn entity_place(&self, host: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
            let Placement::Multi(writes) = &event.placement else {
                panic!("complete footprint")
            };
            assert_eq!(event.pos, BlockPos::new(5, 72, 4));
            let level = host.level("minecraft:overworld").unwrap();
            for write in writes {
                let pos = [write.pos.x, write.pos.y, write.pos.z];
                assert_eq!(level.block_state(host, pos), Some(write.state));
                self.0.writes.lock().unwrap().push(pos);
            }
            if self.0.veto {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    #[mod_api::sunrise_part]
    impl BlockPart for Probe {
        fn set_placed_by(&self, host: &mut dyn ServerHost, event: &mut SetPlacedByEvent<'_>) {
            assert_eq!(event.pos, ORIGIN);
            self.placed_by.fetch_add(1, Ordering::Relaxed);
            if self.destroy {
                let drops = event
                    .level
                    .destroy_block(host, ABOVE, true, None, None, self.collect)
                    .unwrap()
                    .expect("captured destruction");
                if self.collect {
                    assert_eq!(drops.len(), 1);
                    assert_eq!(drops[0].count(), 1);
                } else {
                    assert!(drops.is_empty());
                }
            } else {
                assert_eq!(
                    event
                        .level
                        .set_block(host, ABOVE, event.state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
        }
    }

    let case = std::env::var(FLAG).unwrap();
    let probe = Probe {
        veto: case.ends_with("-veto"),
        destroy: case.starts_with("destroy-"),
        collect: case == "destroy-collect",
        placed_by: Default::default(),
        writes: Default::default(),
    };
    let listened = case != "destroy-no-listener";
    let mut entries = vec![ModEntry::new("placement_writes", Box::new(probe.clone()))];
    if listened {
        entries.push(ModEntry::new(
            "placement_veto",
            Box::new(Listen(probe.clone())),
        ));
    }
    let mods: mod_api::Mods = entries.into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            loaded(game);
            let level = game.level(game.dim.id()).unwrap();
            let player = game.player(13).unwrap();
            let air = api_state(game, "minecraft:air");
            let wither = case.starts_with("wither-");
            let skull = api_state(game, "minecraft:wither_skeleton_skull");
            let soul = api_state(game, "minecraft:soul_sand");
            let wool = api_state(game, "minecraft:white_wool");
            let pattern = [
                ([4, 72, 4], skull),
                ([6, 72, 4], skull),
                ([4, 71, 4], soul),
                ([5, 71, 4], soul),
                ([6, 71, 4], soul),
                ([5, 70, 4], soul),
            ];
            if wither {
                game.ui.difficulty = 2;
                for (pos, state) in pattern {
                    assert_eq!(
                        level.set_block(game, pos, state, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                }
            }
            if probe.destroy {
                assert_eq!(
                    level.set_block(game, ABOVE, wool, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }
            let before_random = <(u64, Option<f64>)>::from(&*game.world.random);
            let mob_count = game.mobs.len();
            let result = player.place_block(
                game,
                level,
                ORIGIN,
                if wither { skull } else { wool },
                BlockFace::Up,
                false,
            );
            assert_eq!(
                result,
                if probe.veto {
                    Err(Refusal::Canceled)
                } else {
                    Ok(())
                }
            );
            let mut expected = vec![ORIGIN];
            if wither {
                expected.extend(pattern.iter().map(|&(pos, _)| pos));
                assert_eq!(game.mobs.len(), mob_count + usize::from(!probe.veto));
                assert_eq!(level.block_state(game, ORIGIN), Some(air));
                for (pos, before) in pattern {
                    assert_eq!(
                        level.block_state(game, pos),
                        Some(if probe.veto { before } else { air })
                    );
                }
            } else {
                expected.push(ABOVE);
                assert_eq!(
                    probe.placed_by.load(Ordering::Relaxed),
                    1,
                    "no replay of Part callback"
                );
                assert_eq!(
                    level.block_state(game, ORIGIN),
                    Some(if probe.veto { air } else { wool })
                );
                assert_eq!(
                    level.block_state(game, ABOVE),
                    Some(if probe.veto == probe.destroy {
                        wool
                    } else {
                        air
                    })
                );
                if probe.destroy {
                    assert!(
                        game.dropped
                            .iter()
                            .all(|item| &*item.stack.item == "minecraft:white_wool")
                    );
                    assert_eq!(
                        game.dropped
                            .iter()
                            .map(|item| item.stack.count)
                            .sum::<u32>(),
                        u32::from(!probe.veto && !probe.collect)
                    );
                    if probe.veto {
                        assert_eq!(
                            <(u64, Option<f64>)>::from(&*game.world.random),
                            before_random
                        );
                    }
                }
            }
            if !listened {
                expected.clear();
            }
            expected.sort_unstable();
            let mut observed = probe.writes.lock().unwrap();
            observed.sort_unstable();
            assert_eq!(*observed, expected);
        })
    });
}

/// Each refusal is its own name and writes nothing.
#[test]
#[ignore = "daily"]
fn each_place_block_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = api_state(game, "minecraft:stone");
        let empty = game.world.block(3, 70, 3);
        let at = [2, 70, 2];
        assert_eq!(
            b.place_block(game, level, at, stone, BlockFace::Up, false),
            Ok(())
        );
        assert_eq!(
            b.place_block(game, level, at, stone, BlockFace::Up, false),
            Err(Refusal::Unchanged),
            "stone already stands there"
        );
        for pos in [[2, 400, 2], [2, -100, 2], [30_000_000, 70, 2]] {
            assert_eq!(
                b.place_block(game, level, pos, stone, BlockFace::Up, true),
                Err(Refusal::OutOfWorld { pos })
            );
        }
        let far = [1600, 70, 1600];
        assert_eq!(
            b.place_block(game, level, far, stone, BlockFace::Up, true),
            Err(Refusal::NotLoaded { pos: far })
        );
        // A mod cannot make a state the world lacks; the game's own check names one.
        let nothing = sunrise_world::registry::BlockStateId(u32::MAX);
        assert_eq!(
            game.player_place_block(b, level, [3, 70, 3], nothing, BlockFace::Up, true),
            Err(Refusal::UnknownState)
        );
        let old =
            mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        assert_eq!(
            b.place_block(game, old, [3, 70, 3], stone, BlockFace::Up, true),
            Err(Refusal::StaleLevel)
        );
        let gone = mod_api::__private::game::player_ref(
            EntityRef::new(
                WorldEpoch::new(b.entity().epoch().get() + 1),
                b.entity().id(),
            ),
            b.uuid(),
        );
        assert_eq!(
            gone.place_block(game, level, [3, 70, 3], stone, BlockFace::Up, true),
            Err(Refusal::Stale)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.dead = true)
            .unwrap();
        assert_eq!(
            b.place_block(game, level, [3, 70, 3], stone, BlockFace::Up, true),
            Err(Refusal::Dead)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
    });
}

/// A level that is open but is not the one in place is `OtherLevel`: the check compares the
/// supplied level with the installed one, not with the player's dimension.
#[test]
#[ignore = "daily"]
fn a_level_not_in_place_is_other_level() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let overworld = game.level("minecraft:overworld").unwrap();
        let stone = api_state(game, "minecraft:stone");
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        assert_eq!(
            b.place_block(game, overworld, [2, 70, 2], stone, BlockFace::Up, true),
            Err(Refusal::OtherLevel)
        );
    });
}

/// On a client it is `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_place_a_block_as_a_player() {
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::execution_role::ExecutionRole;

    with_game(|game| {
        let level = mod_api::LevelRef::new(WorldEpoch::new(0), game.dim.key());
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        let stone = api_state(game, "minecraft:stone");
        let role = game.common.options.execution_role;
        game.common.options.execution_role = ExecutionRole::Client;
        assert_eq!(
            ghost.place_block(game, level, [2, 70, 2], stone, BlockFace::Up, true),
            Err(Refusal::Client)
        );
        game.common.options.execution_role = role;
    });
}

/// `BlockBehaviour.onPlace` runs for every write, so a part on glass that finds a signal beside
/// it lights itself within the write (`CopperBulbBlock.onPlace`): by a level write and by a
/// player's placement, with the lit state in the level before any tick. Glass that is not
/// powered stays, and a block with no part is unchanged.
#[test]
#[ignore = "daily"]
fn a_part_lights_itself_in_on_place_for_a_level_write_and_a_player_placement() {
    use mod_platform::__private::part_tests::{block_on_place_game_thread, block_on_places};

    // The fixture installs its mod for the process: run it in a child of its own.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::native_player_place_tests::a_part_lights_itself_in_on_place_for_a_level_write_and_a_player_placement",
        "SUNRISE_BLOCK_ON_PLACE_CHILD",
        &["place"],
    ) {
        return;
    }
    block_on_place_game_thread();
    with_guests(|game| {
        loaded(game);
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (glass, lit, plain) = (
            state("minecraft:glass"),
            state("minecraft:white_stained_glass"),
            state("minecraft:black_stained_glass"),
        );
        let air = api_state(game, "minecraft:air").block();
        let dimension = game.dim.id().to_owned();
        for z in [2, 6, 10, 14] {
            game.world
                .set(4, 70, z, state("minecraft:redstone_block"))
                .unwrap();
        }
        assert!(block_on_places().is_empty());

        // A level write beside the redstone block: lit at once, with no tick run.
        assert!(game.native_set_block(&dimension, [5, 70, 2], glass, 3));
        assert_eq!(game.world.block(5, 70, 2), lit);
        assert_eq!(block_on_places(), [([5, 70, 2], air, false)]);

        // A player's placement beside the redstone block.
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let api_glass = api_state(game, "minecraft:glass");
        assert_eq!(
            b.place_block(game, level, [5, 70, 6], api_glass, BlockFace::Up, true),
            Ok(())
        );
        assert_eq!(game.world.block(5, 70, 6), lit);
        assert_eq!(block_on_places(), [([5, 70, 6], air, false)]);

        // A block with no part is unchanged beside the same redstone block.
        assert!(game.native_set_block(&dimension, [5, 70, 10], plain, 3));
        assert_eq!(game.world.block(5, 70, 10), plain);
        assert!(block_on_places().is_empty());

        // The part hears a write with no signal and leaves the glass as it is.
        assert!(game.native_set_block(&dimension, [8, 70, 14], glass, 3));
        assert_eq!(game.world.block(8, 70, 14), glass);
        assert_eq!(block_on_places(), [([8, 70, 14], air, false)]);
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
