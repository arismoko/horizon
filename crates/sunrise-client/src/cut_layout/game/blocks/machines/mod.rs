#[cfg(test)]
use sunrise_core::geometry::Direction;
#[cfg(test)]
use sunrise_core::position::BlockPos;
#[cfg(test)]
use sunrise_gameplay_common::hoppers::Pos;
#[cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
use sunrise_level::containers::{BlockEntity, HOPPER_SLOTS};
#[cfg(test)]
use sunrise_world::registry::BlockStateId;
#[cfg(test)]
pub(crate) mod can_place_item_tests;
#[cfg(test)]
pub(crate) mod contents_changed_tests;
#[cfg(test)]
pub(crate) mod contents_watch_tests;
#[cfg(test)]
pub(crate) mod tank_reopen_tests;

#[cfg(test)]
pub(crate) mod brewing_fuel_transfer_tests {
    use super::*;
    use mod_api::HostTransaction;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_level::brewing::BrewingStand;

    struct PrepareProbe {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fill: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PrepareProbe {
        fn prepare_hopper_transfer(
            &self,
            host: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::PrepareHopperTransferEvent,
        ) {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fill {
                let level = host.level("minecraft:overworld").unwrap();
                let items = level
                    .capability(host, &mod_api::ITEMS, [8, 64, 8], None)
                    .unwrap();
                let stack = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stone").unwrap(),
                    1,
                )
                .unwrap();
                let resource = mod_api::ItemResource::of(&stack);
                host.transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 0, &resource, 1), Ok(1));
                    Ok::<_, ()>(())
                })
                .unwrap();
            }
        }
    }

    fn check_prepare_transfer(fill: bool) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "prepare_probe",
            Box::new(PrepareProbe {
                calls: calls.clone(),
                fill,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let pos = (8, 64, 8);
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:hopper")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![None; HOPPER_SLOTS]));
                if fill {
                    let chest = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:chest")
                        .unwrap();
                    game.world.set(8, 63, 8, chest).unwrap();
                    game.world
                        .block_entities
                        .insert((8, 63, 8), BlockEntity::Chest(vec![None; 27]));
                }
                game.hopper_try_move_items(pos);
                assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
                if fill {
                    let containers = game.containers();
                    let target = containers.view((8, 63, 8), false).unwrap();
                    assert_eq!(
                        containers.get(&target, 0).unwrap().item.as_ref(),
                        "minecraft:stone"
                    );
                    assert_eq!(containers.get(&target, 0).unwrap().count, 1);
                    assert!(containers.is_empty(&containers.view(pos, false).unwrap()));
                    assert_eq!(game.server.hoppers[&pos].cooldown, 8);
                }
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn empty_hopper_dispatches_prepare_transfer() {
        check_prepare_transfer(false);
    }

    #[test]
    #[ignore = "daily"]
    fn prepare_transfer_inventory_write_is_ejected() {
        check_prepare_transfer(true);
    }

    #[test]
    #[ignore = "daily"]
    fn hopper_brewing_fuel_transfer_respects_stack_components() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let hopper_pos = (8, 64, 8);
            let stand_pos = (9, 64, 8);
            for (pos, block) in [
                (hopper_pos, "minecraft:hopper"),
                (stand_pos, "minecraft:brewing_stand"),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            }
            let default = ItemStack::new("minecraft:blaze_powder", 2);
            let mut custom = ItemStack::new("minecraft:stone", 2);
            custom.data_mut().set_loot_component(
                "minecraft:brewing_fuel",
                &Some(
                    sunrise_core::json_nbt::json_to_tag(
                        &serde_json::json!({"uses": 7, "speed_multiplier": 2.5}),
                    )
                    .unwrap(),
                ),
            );
            let mut removed = default.clone();
            removed.data_mut().set_other(
                "!minecraft:brewing_fuel",
                Some(sunrise_core::nbt::Tag::Compound(Default::default())),
            );
            for (stack, accepted) in [
                (default, true),
                (custom, true),
                (removed, false),
                (ItemStack::new("minecraft:stone", 2), false),
            ] {
                game.world
                    .block_entities
                    .insert(hopper_pos, BlockEntity::Chest(vec![Some(stack); 5]));
                game.world.block_entities.insert(
                    stand_pos,
                    BlockEntity::Brewing(Box::new(BrewingStand::new())),
                );
                let containers = game.containers();
                let hopper = containers.view(hopper_pos, false).unwrap();
                let stand = containers.view(stand_pos, false).unwrap();
                let fuel = containers.get(&hopper, 0).unwrap();
                assert_eq!(
                    containers.can_place(
                        &stand,
                        4,
                        fuel,
                        sunrise_gameplay_common::game::machine_containers::PlacementQuery::Offer(&mut sunrise_gameplay_common::game::machine_containers::PlacementAttempt::default()),
                    ),
                    accepted
                );
                assert_eq!(
                    game.eject_into(hopper_pos, &hopper, &stand, Direction::West),
                    accepted
                );
                let containers = game.containers();
                assert_eq!(
                    containers.get(&hopper, 0).unwrap().count,
                    if accepted { 1 } else { 2 }
                );
                assert_eq!(
                    containers.get(&stand, 4).map(|s| s.count),
                    accepted.then_some(1)
                );
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod furnace_fuel_transfer_tests {
    use super::*;
    use sunrise_items::item_data::parse_stack;
    use sunrise_level::containers::{Furnace, FurnaceKind};
    use {
        sunrise_gameplay::game::game_tests::tests::server_game,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    #[test]
    #[ignore = "daily"]
    fn a_side_hopper_respects_the_furnace_fuel_patch_and_bucket_rule() {
        if run_fresh_cases(
            "cut_layout::game::server_side::machines::furnace_fuel_transfer_tests::a_side_hopper_respects_the_furnace_fuel_patch_and_bucket_rule",
            "SUNRISE_FURNACE_HOPPER_CASE",
            &["side"],
        ) {
            return;
        }
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let hopper = (8, 64, 8);
        let furnace = (9, 64, 8);
        let registry = &game.common.pack.assets.registry;
        let mut hopper_state = registry
            .state(registry.default_state("minecraft:hopper").unwrap())
            .unwrap()
            .clone();
        hopper_state
            .properties
            .insert("facing".into(), "east".into());
        game.common
            .world
            .set(
                hopper.0,
                hopper.1,
                hopper.2,
                registry.resolve(&hopper_state).unwrap(),
            )
            .unwrap();
        game.common
            .world
            .set(
                furnace.0,
                furnace.1,
                furnace.2,
                registry.default_state("minecraft:furnace").unwrap(),
            )
            .unwrap();
        for line in include_str!("furnace_hopper.tsv")
            .lines()
            .filter(|line| !line.starts_with('#'))
        {
            let [case, offered, held, moves] = line.split('\t').collect::<Vec<_>>()[..] else {
                panic!("invalid furnace hopper case: {line}");
            };
            let stack = parse_stack(offered).unwrap().unwrap();
            let held = parse_stack(held).unwrap();
            let moves: bool = moves.parse().unwrap();
            let mut hopper_slots = vec![None; HOPPER_SLOTS];
            hopper_slots[0] = Some(stack.clone());
            game.world
                .block_entities
                .insert(hopper, BlockEntity::Chest(hopper_slots));
            game.world.block_entities.insert(
                furnace,
                BlockEntity::Furnace(Furnace {
                    kind: FurnaceKind::Furnace,
                    items: [None, held.clone(), None],
                    lit_remaining: 0,
                    lit_total: 0,
                    cooking_timer: 0,
                    cooking_total: 200,
                    speed: 1.0,
                    recipes_used: Default::default(),
                }),
            );
            game.server.hoppers.remove(&hopper);

            game.hopper_try_move_items(hopper);

            let containers = game.containers();
            let source = containers.view(hopper, false).unwrap();
            let destination = containers.view(furnace, false).unwrap();
            let mut source_left = stack.clone();
            source_left.count -= u32::from(moves);
            assert_eq!(
                containers.get(&source, 0),
                Some(&source_left),
                "{case}: hopper"
            );
            let target = if moves {
                let mut after = held.unwrap_or_else(|| {
                    let mut stack = stack.clone();
                    stack.count = 0;
                    stack
                });
                after.count += 1;
                Some(after)
            } else {
                held
            };
            assert_eq!(
                containers.get(&destination, 1),
                target.as_ref(),
                "{case}: fuel slot"
            );
            assert!(
                containers.get(&destination, 0).is_none()
                    && containers.get(&destination, 2).is_none(),
                "{case}: side did not touch the other slots"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod part_front_tests {
    use super::super::native_entity_refs::tests::{declared_part_root, with_game_at};
    use mod_api::BlockUpdateFlags;
    use std::sync::Arc;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_level::containers::BlockEntity;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// A crafter facing west into a pipe that hands on east, back into the crafter: the four
    /// sticks land in its grid, and using up the planks takes none of them.
    #[test]
    #[ignore = "daily"]
    fn a_part_handing_a_crafters_output_back_into_its_grid_loses_none() {
        let Some(root) = declared_part_root(
            "cut_layout::game::server_side::machines::part_front_tests::a_part_handing_a_crafters_output_back_into_its_grid_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = sunrise_world::chunk::BlockState {
                name: "minecraft:crafter".into(),
                properties: [("orientation".into(), "west_up".into())]
                    .into_iter()
                    .collect(),
            };
            let crafter = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], crafter, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get_mut(&(2, 64, 8)) {
                c.items[3] = Some(stack("minecraft:oak_planks", 1));
                c.items[6] = Some(stack("minecraft:oak_planks", 1));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), crafter)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Crafter(c)) => c
                    .items
                    .iter()
                    .flatten()
                    .map(|s| (s.item.clone(), s.count))
                    .collect::<Vec<_>>(),
                _ => panic!("no crafter"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (vec![("minecraft:stick".into(), 4)], 0),
                "a pipe routing output back into the grid must conserve four crafted sticks"
            );
        });
    }

    /// A dropper facing west into a pipe that hands on east, back into the dropper: the item
    /// goes round, and taking it from its slot as it is then keeps both.
    #[test]
    #[ignore = "daily"]
    fn a_part_handing_a_droppers_item_back_into_it_loses_none() {
        let Some(root) = declared_part_root(
            "cut_layout::game::server_side::machines::part_front_tests::a_part_handing_a_droppers_item_back_into_it_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = sunrise_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "west".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], dropper, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 8))
            {
                items[0] = Some(stack("minecraft:stick", 2));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), dropper)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("no dropper"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (2, 0),
                "a pipe routing back into the source must conserve its items"
            );
        });
    }

    /// A dropper and a crafter facing a mod's item part (NeoForge's `dropperInsertHook` and
    /// `insertCrafterOutput`): a truthful part, a pipe into the chest east of it, takes what
    /// they put out in a container's place; a part answering more than it was offered has its
    /// move undone, so the dropper keeps its item and the crafter's result is dispensed.
    #[test]
    #[ignore = "daily"]
    fn a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them() {
        let Some(root) = declared_part_root(
            "cut_layout::game::server_side::machines::part_front_tests::a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state =
                |game: &sunrise_gameplay::game::ServerGame<'_>, name: &str, prop: (&str, &str)| {
                    let state = sunrise_world::chunk::BlockState {
                        name: name.to_owned(),
                        properties: [(prop.0.to_owned(), prop.1.to_owned())]
                            .into_iter()
                            .collect(),
                    };
                    game.pack.assets.registry.resolve(&state).unwrap()
                };
            let dropper = state(game, "minecraft:dropper", ("facing", "east"));
            let crafter = state(game, "minecraft:crafter", ("orientation", "east_up"));
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let chest_items = |game: &sunrise_gameplay::game::ServerGame<'_>, z| match game
                .world
                .block_entities
                .get(&(2, 64, z))
            {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("no chest"),
            };
            for (z, part, taken) in [(0, "test:pipe", true), (4, "test:greedy", false)] {
                for (x, block) in [(0, dropper), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Chest(items)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    items[4] = Some(stack("minecraft:stick", 2));
                }
                game.dispense_from(BlockPos::new(0, 64, z), dropper)
                    .unwrap();
                let left = match game.world.block_entities.get(&(0, 64, z)) {
                    Some(BlockEntity::Chest(items)) => items[4].as_ref().map_or(0, |s| s.count),
                    _ => panic!("no dropper"),
                };
                assert_eq!(
                    (left, chest_items(game, z)),
                    if taken { (1, 1) } else { (2, 0) },
                    "{part}"
                );

                // Two planks make four sticks.
                let z = z + 2;
                for (x, block) in [(0, crafter), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Crafter(c)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    c.items[0] = Some(stack("minecraft:oak_planks", 1));
                    c.items[3] = Some(stack("minecraft:oak_planks", 1));
                }
                let items = game.dropped.len();
                game.dispense_from(BlockPos::new(0, 64, z), crafter)
                    .unwrap();
                let spawned = game.dropped.len() - items;
                assert_eq!(
                    (chest_items(game, z), spawned),
                    if taken { (4, 0) } else { (0, 1) },
                    "{part}"
                );
            }
        });
    }
}

#[cfg(test)]
#[path = "hopper_cart_rider_tests.rs"]
pub(crate) mod hopper_cart_rider_tests;

#[cfg(test)]
pub(crate) mod hopper_cart_transfer_tests {
    use super::super::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::{
        boats_common::Boat,
        minecarts::{CartKind, Minecart},
    };

    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_items::items::ItemStack;
    use sunrise_level::containers::BlockEntity;

    #[test]
    #[ignore = "daily"]
    fn chest_boat_automation_does_not_award_player_loot() {
        use std::sync::Arc;
        use sunrise_sim::advancements::Tree;
        use sunrise_world::loot::PackOverlay;

        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = None;
            let files = vec![(
            "advancement/parity:boat_loot".into(),
            vec![br#"{"criteria":{"loot":{"trigger":"minecraft:player_generates_container_loot","conditions":{"loot_tables":"minecraft:chests/spawn_bonus_chest"}}}}"#.to_vec()],
        )];
            game.adv.tree = Arc::new(Tree::load_with(
                &game.options.resources.join("data"),
                &PackOverlay {
                    files: &files,
                    ..PackOverlay::default()
                },
            ));
            game.adv.loaded = true;
            let ai = game.adv.tree.by_id["parity:boat_loot"];
            for player in [false, true] {
                game.server.next_entity += 1;
                let id = game.server.next_entity;
                game.boats
                    .push(Boat::new(id, "oak".into(), Vec3::new(8.5, 65.0, 8.5), 0.0).with_chest());
                game.persist.raw.insert(
                    id,
                    Tag::compound([
                        (
                            "LootTable",
                            Tag::String("minecraft:chests/spawn_bonus_chest".into()),
                        ),
                        ("LootTableSeed", Tag::Long(73)),
                    ]),
                );
                let i = game.boats.len() - 1;
                game.as_guest(12, |game| {
                    game.unpack_boat_loot(i, player);
                    assert_eq!(game.adv.progress.done(&game.adv.tree.list[ai]), player);
                    // Opening a container automation already unpacked awards nothing.
                    game.unpack_boat_loot(i, true);
                    assert_eq!(game.adv.progress.done(&game.adv.tree.list[ai]), player);
                })
                .unwrap();
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn chest_boat_with_items_drops_pending_table_without_rolling() {
        with_game(|game| {
            let sentinel = Some(ItemStack::new("minecraft:diamond", 17));
            for open in [false, true] {
                for occupied in [1, 24, 27] {
                    game.server.next_entity += 1;
                    let id = game.server.next_entity;
                    let mut boat =
                        Boat::new(id, "oak".into(), Vec3::new(8.5, 65.0, 8.5), 0.0).with_chest();
                    if occupied > 1 {
                        boat.items.fill(sentinel.clone());
                        if occupied == 24 {
                            for slot in [0, 13, 26] {
                                boat.items[slot] = None;
                            }
                        }
                    } else {
                        boat.items[12] = sentinel.clone();
                    }
                    let key = (
                        i32::MAX - 15,
                        sunrise_gameplay_common::game::minecarts::CART_KEY_Y,
                        id as i32,
                    );
                    if open {
                        game.world
                            .block_entities
                            .insert(key, BlockEntity::Chest(boat.items.clone()));
                        // The menu copy is authoritative, even if the entity is stale.
                        boat.items.fill(None);
                    }
                    game.boats.push(boat);
                    game.persist.raw.insert(
                        id,
                        Tag::compound([
                            (
                                "LootTable",
                                Tag::String("minecraft:chests/spawn_bonus_chest".into()),
                            ),
                            ("LootTableSeed", Tag::Long(73)),
                        ]),
                    );
                    let items = game
                        .with_entity_container(id, |g, _| {
                            g.world.block_entities.get(&key).unwrap().slots().to_vec()
                        })
                        .unwrap();
                    assert_eq!(items[12], sentinel);
                    // Sunrise-only mixed state: never fill even the free slots.
                    assert_eq!(items.iter().flatten().count(), occupied);
                    assert!(
                        items
                            .iter()
                            .flatten()
                            .all(|item| Some(item) == sentinel.as_ref())
                    );
                    assert!(game.persist.raw[&id].get("LootTable").is_none());
                    assert!(game.persist.raw[&id].get("LootTableSeed").is_none());
                    assert_eq!(game.boats.last().unwrap().items, items);
                    game.with_entity_container(id, |g, _| {
                        if let Some(BlockEntity::Chest(items)) =
                            g.world.block_entities.get_mut(&key)
                        {
                            items.fill(None);
                        }
                    })
                    .unwrap();
                    game.with_entity_container(id, |g, _| {
                        assert!(
                            g.world
                                .block_entities
                                .get(&key)
                                .unwrap()
                                .slots()
                                .iter()
                                .all(Option::is_none)
                        );
                    })
                    .unwrap();
                    game.release_vehicle_container(id);
                }
            }
        });
    }

    /// Issue #8668: a hopper minecart takes one item a tick from the container
    /// above. `MinecartHopper.tick` has no transfer cooldown in 26.3: it calls
    /// `tryConsumeItems` every tick, and `consumedItemThisFrame` caps a tick at
    /// one item even though `makeStepAlongTrack` may try too. A whole stack a
    /// tick (the old behaviour) or a block hopper's eight-tick cooldown both
    /// fail this.
    #[test]
    #[ignore = "daily"]
    fn hopper_cart_takes_one_item_a_tick() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.server.streaming.forced.insert((0, 0));
            let rail = game
                .pack
                .assets
                .registry
                .default_state("minecraft:rail")
                .unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            // A rail holds the cart still; the chest sits in the block the cart
            // pulls from (`getLevelY` is y + 0.5, so the block above is y + 1.5).
            game.world.set(8, 64, 8, rail).unwrap();
            game.world.set(8, 65, 8, chest).unwrap();
            game.world.block_entities.insert(
                (8, 65, 8),
                BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 27]),
            );
            game.server.next_entity += 1;
            let id = game.server.next_entity;
            let mut cart = Minecart::new(id, CartKind::Hopper, Vec3::new(8.5, 64.0625, 8.5));
            cart.enabled = true;
            game.carts.push(cart);
            let count = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let in_chest: u32 = match game.world.block_entities.get(&(8, 65, 8)) {
                    Some(BlockEntity::Chest(s)) => s.iter().flatten().map(|s| s.count).sum(),
                    _ => 0,
                };
                let in_cart: u32 = game
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .unwrap()
                    .items
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum();
                (in_chest, in_cart)
            };
            let (before_chest, before_cart) = count(game);
            for _ in 0..20 {
                game.tick_carts();
            }
            let (after_chest, after_cart) = count(game);
            assert_eq!(before_chest, 27 * 64);
            assert_eq!(before_cart, 0);
            assert_eq!(before_chest - after_chest, after_cart - before_cart);
            assert_eq!(
                after_cart, 20,
                "one item a tick, not a stack nor eight ticks"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod pot_hopper_insertion_tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_sim::save;
    use {
        sunrise_gameplay::game::game_tests::tests::server_game,
        sunrise_gameplay_common::game::spill_common as spill,
    };

    fn count(game: &sunrise_gameplay::game::ServerGame<'_>, pos: Pos, slot: usize) -> u32 {
        let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
            panic!("actual command-created inventory missing at {pos:?}");
        };
        slots[slot].as_ref().map_or(0, |item| item.count)
    }
    fn pot(game: &sunrise_gameplay::game::ServerGame<'_>, z: i32) -> (Tag, ItemStack) {
        let tag = spill::saved_entity(&game.world, [5, 200, z]).expect("actual saved pot");
        let item = save::item_from_nbt(tag.get("item").expect("positive actual item")).unwrap();
        (tag, item)
    }

    #[test]
    #[ignore = "daily"]
    fn actual_hopper_ticks_insert_into_the_saved_pot_slot() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        let mut initial = Vec::new();
        for (z, id, target) in [
            (4, "wheat", "decorated_pot"),
            (8, "carrot", "decorated_pot"),
            (12, "wheat", "chest"),
        ] {
            let command = format!(
                "setblock 4 200 {z} hopper[facing=east,enabled=true]{{Items:[{{Slot:0b,id:\"minecraft:{id}\",count:40,components:{{\"minecraft:custom_data\":{{h282:1}}}}}},{{Slot:4b,id:\"minecraft:diamond\",count:2}}],TransferCooldown:0}}"
            );
            assert!(game.run_command(&command));
            let content = if target == "decorated_pot" {
                "item:{id:\"minecraft:wheat\",count:3,components:{\"minecraft:custom_data\":{h282:1}}}"
            } else {
                "Items:[{Slot:0b,id:\"minecraft:wheat\",count:3,components:{\"minecraft:custom_data\":{h282:1}}}]"
            };
            assert!(game.run_command(&format!("setblock 5 200 {z} {target}{{{content}}}")));
            assert_eq!(count(&game, (4, 200, z), 0), 40);
            assert_eq!(count(&game, (4, 200, z), 4), 2);
            if target == "decorated_pot" {
                assert!(!game.world.block_entities.contains_key(&(5, 200, z)));
                assert!(
                    game.containers().view((5, 200, z), false).is_none(),
                    "generic discovery stays unchanged"
                );
                let (tag, item) = pot(&game, z);
                assert_eq!(item.count, 3);
                assert_eq!(&*item.item, "minecraft:wheat");
                initial.push((z, tag, item));
            }
        }
        assert_eq!(count(&game, (5, 200, 12), 0), 3);
        let start = game.world.game_time;
        let mut failures = Vec::new();
        for elapsed in 1..=217 {
            game.world.game_time = start + elapsed;
            for z in [4, 8, 12] {
                game.tick_hopper((4, 200, z));
            }
            if [1, 9, 217].contains(&elapsed) {
                let n = ((elapsed - 1) / 8 + 1) as u32;
                assert_eq!(
                    count(&game, (4, 200, 12), 0),
                    40 - n,
                    "ordinary hopper cadence"
                );
                assert_eq!(count(&game, (5, 200, 12), 0), 3 + n);
                assert_eq!(count(&game, (4, 200, 8), 0), 40, "different item rejected");
                assert_eq!(pot(&game, 8).1.count, 3);
                for z in [4, 8, 12] {
                    assert_eq!(count(&game, (4, 200, z), 4), 2);
                }
                let (mut saved, item) = pot(&game, 4);
                let mut before = initial[0].1.clone();
                if let (Tag::Compound(a), Tag::Compound(b)) = (&mut saved, &mut before) {
                    a.remove(&"item".into());
                    b.remove(&"item".into());
                }
                assert_eq!(saved, before, "raw metadata preserved");
                assert!(item.same_item_same_components(&initial[0].2));
                let donor = count(&game, (4, 200, 4), 0);
                assert_eq!(donor + item.count, 43, "actual conserved inventory");
                assert!(game.containers().view((5, 200, 4), false).is_none());
                if donor != 40 - n || item.count != 3 + n {
                    failures.push((elapsed, donor, item.count));
                }
            }
        }
        assert_eq!(game.world.game_time - start, 217);
        assert!(
            failures.is_empty(),
            "actual hopper/pot transfer missing: {failures:?}"
        );
    }
    #[test]
    #[ignore = "daily"]
    fn actual_hopper_pulls_from_the_saved_pot_slot() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(
            game.run_command(
                "setblock 5 201 4 decorated_pot{item:{id:\"minecraft:wheat\",count:2}}"
            )
        );
        assert!(
            game.run_command(
                "setblock 5 200 4 hopper[facing=down,enabled=true]{TransferCooldown:0}"
            )
        );
        let start = game.world.game_time;
        for elapsed in 1..=9 {
            game.world.game_time = start + elapsed;
            game.tick_hopper((5, 200, 4));
        }
        let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&(5, 200, 4)) else {
            panic!("hopper inventory missing");
        };
        let held: u32 = slots.iter().flatten().map(|s| s.count).sum();
        assert_eq!(held, 2, "the hopper took both items");
        let tag = spill::saved_entity(&game.world, [5, 201, 4]).expect("saved pot");
        assert!(tag.get("item").is_none(), "the pot is empty");
    }
}

#[cfg(test)]
#[path = "machines_pot_signal_tests.rs"]
pub(crate) mod pot_signal_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::GameGameExt1;

    /// `ShelfBlockEntity.setChanged` (no arguments) posts `BLOCK_ACTIVATE`:
    /// a container writing a shelf's slot is a block activation.
    #[test]
    #[ignore = "daily"]
    fn a_container_writing_a_shelf_posts_block_activate() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let key = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_shelf")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Shelf(Box::default()));
        let view = game
            .containers()
            .view(key, false)
            .expect("the shelf's view");
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = Some(Vec::new()));

        game.view_changed(&view);

        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
            assert!(
                e.borrow().as_ref().unwrap().contains(&(
                    "minecraft:block_activate",
                    [2.5, 4.5, 3.5],
                    None
                )),
                "{:?}",
                e.borrow()
            );
        });
    }

    fn hopper_facing(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        facing: &str,
        enabled: bool,
    ) -> BlockStateId {
        let registry = &game.common.pack.assets.registry;
        let mut state = registry
            .state(registry.default_state("minecraft:hopper").unwrap())
            .unwrap()
            .clone();
        state.properties.insert("facing".into(), facing.into());
        state
            .properties
            .insert("enabled".into(), enabled.to_string());
        registry.resolve(&state).unwrap()
    }
    fn place_hopper(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        pos: Pos,
        facing: &str,
        enabled: bool,
        item: Option<ItemStack>,
    ) {
        let state = hopper_facing(game, facing, enabled);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        let mut slots = vec![None; HOPPER_SLOTS];
        slots[0] = item;
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(slots));
    }
    fn hopper_count(game: &sunrise_gameplay::game::ServerGame<'_>, pos: Pos) -> u32 {
        match game.world.block_entities.get(&pos) {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum(),
            other => panic!("no hopper at {pos:?}: {other:?}"),
        }
    }
    fn tick_block_entities(game: &mut sunrise_gameplay::game::ServerGame<'_>, time: i64) {
        game.world.game_time = time;
        game.tick_block_entities().unwrap();
    }

    /// An empty hopper with nothing above sleeps after its tick, and one a
    /// redstone signal turned off sleeps too. The real scheduler still advances
    /// a sleeping hopper's transfer state, so a feeder that runs after its slot
    /// sets the vanilla 7-tick cooldown, in both ticker orders.
    #[test]
    #[ignore = "daily"]
    fn an_idle_hopper_sleeps_and_wakes_for_what_it_could_take() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::machines::tests::an_idle_hopper_sleeps_and_wakes_for_what_it_could_take",
            || {
                let mut resources = None;
                let mut game = server_game(&mut resources);
                game.build_registries();
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(8.5, 65.0, 8.5),
                    )
                    .unwrap(),
                );
                game.update_chunk_tickets();
                assert!(game.block_ticking_area()(1, 1));
                let start = game.world.game_time;
                let stone = || Some(ItemStack::new("minecraft:stone", 1));

                // The receiver ticks first and goes idle; the side feeder is
                // placed later, so it runs after the receiver's slot.
                let side_receiver = (1, 64, 1);
                let side_feeder = (2, 64, 1);
                place_hopper(&mut game, side_receiver, "down", true, None);
                tick_block_entities(&mut game, start + 1);
                assert!(
                    game.world.block_entities.is_hopper_asleep(&side_receiver),
                    "an empty hopper with no source above sleeps"
                );
                place_hopper(&mut game, side_feeder, "west", true, stone());
                tick_block_entities(&mut game, start + 2);
                assert_eq!(
                    hopper_count(&game, side_receiver),
                    1,
                    "the side feeder fills the woken receiver"
                );
                assert_eq!(
                    game.hopper_cooldown(side_receiver),
                    7,
                    "the receiver ticked before the feeder in this tick"
                );

                // Feeder first, receiver second: the receiver's own tick
                // decrements the 8 the feeder set, and still ends at 7.
                let early_feeder = (6, 64, 1);
                let later_receiver = (5, 64, 1);
                place_hopper(&mut game, early_feeder, "west", true, stone());
                place_hopper(&mut game, later_receiver, "down", true, None);
                tick_block_entities(&mut game, start + 3);
                assert_eq!(hopper_count(&game, later_receiver), 1);
                assert_eq!(
                    game.hopper_cooldown(later_receiver),
                    7,
                    "the receiver ticked after the feeder in this tick"
                );

                // A powered receiver fed from above: it sleeps until the
                // feeder's insert wakes it, after its own slot.
                let powered = (8, 64, 1);
                let above_feeder = (8, 65, 1);
                place_hopper(&mut game, powered, "down", false, None);
                tick_block_entities(&mut game, start + 4);
                assert!(
                    game.world.block_entities.is_hopper_asleep(&powered),
                    "a powered hopper sleeps"
                );
                place_hopper(&mut game, above_feeder, "down", true, stone());
                tick_block_entities(&mut game, start + 5);
                assert_eq!(hopper_count(&game, powered), 1);
                assert_eq!(
                    game.hopper_cooldown(powered),
                    7,
                    "a powered receiver fed above keeps the 7-tick cooldown"
                );
                assert!(!game.world.block_entities.is_asleep(&powered));
            },
        );
    }
}

#[cfg(test)]
#[path = "machines_chest_cat_tests.rs"]
pub(crate) mod machines_chest_cat_tests;

#[cfg(test)]
pub(crate) mod can_take_tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay::game::machines::{TakeAsk, TakeAttempt};
    use sunrise_level::containers::ChiseledBookshelf;

    /// `ChiseledBookShelfBlockEntity.canTakeItem`: a hopper takes a book only
    /// when it has a free slot or a matching stack with room.
    #[test]
    #[ignore = "daily"]
    fn a_full_hopper_cannot_take_a_book_from_a_chiseled_shelf() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let shelf_pos = (2, 4, 3);
        let hopper_pos = (2, 5, 3);
        let shelf_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chiseled_bookshelf")
            .unwrap();
        let hopper_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        game.common
            .world
            .set(shelf_pos.0, shelf_pos.1, shelf_pos.2, shelf_state)
            .unwrap();
        game.common
            .world
            .set(hopper_pos.0, hopper_pos.1, hopper_pos.2, hopper_state)
            .unwrap();
        let book = ItemStack::new("minecraft:book", 1);
        let mut shelf = ChiseledBookshelf::default();
        shelf.items[0] = Some(book.clone());
        game.world
            .block_entities
            .insert(shelf_pos, BlockEntity::ChiseledBookshelf(Box::new(shelf)));
        let stone = ItemStack::new("minecraft:stone", 64);
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![Some(stone.clone()); 5]));
        let shelf_view = game.containers().view(shelf_pos, false).expect("shelf");
        let full_hopper = game.containers().view(hopper_pos, false).expect("hopper");
        assert!(
            !game.containers().can_take_into(
                &shelf_view,
                0,
                &book,
                Direction::Down,
                &full_hopper,
                TakeAsk {
                    game: &game,
                    attempt: &mut TakeAttempt::default(),
                },
            ),
            "a full hopper takes no book"
        );
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![None; 5]));
        let empty_hopper = game.containers().view(hopper_pos, false).expect("hopper");
        assert!(
            game.containers().can_take_into(
                &shelf_view,
                0,
                &book,
                Direction::Down,
                &empty_hopper,
                TakeAsk {
                    game: &game,
                    attempt: &mut TakeAttempt::default(),
                },
            ),
            "an empty hopper takes the book"
        );
    }

    /// A non-container block entity above a hopper (such as a sign holding lava)
    /// has no slots, so `view` returns `None`. The hopper does not treat it as a
    /// container, allowing item entities resting on the rim to be picked up.
    #[test]
    #[ignore = "daily"]
    fn non_container_block_entity_above_hopper_does_not_block_item_suction() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let hopper_pos = (2, 4, 3);
        let sign_pos = (2, 5, 3);
        let hopper_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        let sign_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_sign")
            .unwrap();
        game.common
            .world
            .set(hopper_pos.0, hopper_pos.1, hopper_pos.2, hopper_state)
            .unwrap();
        game.common
            .world
            .set(sign_pos.0, sign_pos.1, sign_pos.2, sign_state)
            .unwrap();
        game.world
            .block_entities
            .insert(hopper_pos, BlockEntity::Chest(vec![None; 5]));
        game.world
            .block_entities
            .insert(sign_pos, BlockEntity::Sign(Default::default()));

        assert!(
            game.containers().view(sign_pos, false).is_none(),
            "a sign is not a container"
        );
        assert!(
            game.containers().hopper_insertion_view(sign_pos).is_none(),
            "a sign has no hopper insertion view"
        );
        let hopper_view = game
            .containers()
            .view(hopper_pos, false)
            .expect("hopper view");
        assert!(
            game.suck_from_container(sign_pos, [2.5, 5.5, 3.5], &hopper_view)
                .is_none(),
            "suck_from_container returns None so hopper falls through to suck_item_entities"
        );
    }
}
#[cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

pub(crate) mod native_items;

pub(crate) mod native_fluids;

pub(crate) mod crafter;
