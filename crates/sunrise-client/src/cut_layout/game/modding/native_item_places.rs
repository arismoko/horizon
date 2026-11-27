#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use mod_api::HostTransaction as _;
    use sunrise_gameplay_common::game::server_side::native_item_places::tests::Bin;
    use sunrise_gameplay_common::game::{
        native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _,
        native_server::NativeServerGameExt1 as _, tables::GameItems as _,
    };

    use sunrise_gameplay_common::game::TryIntoApi;

    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;

    use sunrise_gameplay_common::game::server_side::native_item_places::tests::offers;

    use sunrise_gameplay::game::server_side::native_item_places::tests::hopper_slot;

    use sunrise_gameplay::game::server_side::native_item_places::tests::hopper_with;

    use sunrise_gameplay::game::server_side::native_item_places::tests::inventories;

    use sunrise_gameplay::game::server_side::native_item_places::tests::menu_of;

    use sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan;

    use sunrise_gameplay::game::server_side::native_item_places::tests::pig;

    use sunrise_gameplay::game::server_side::native_item_places::tests::saved_mob_attribute;

    use sunrise_gameplay::game::server_side::native_item_places::tests::{TradeRefused, trade};

    use sunrise_gameplay::game::server_side::native_item_places::tests::traders;

    use sunrise_gameplay::game::server_side::native_item_places::tests::with_cauldron;

    use sunrise_gameplay::game::server_side::native_item_places::tests::write;

    use sunrise_gameplay::game::server_side::native_item_places::tests::zombie;

    use sunrise_level::containers::BlockEntity;

    use super::super::native_entity_refs::tests::{with_game, with_resident_game};

    use mod_api::ServerHost;

    use mod_api::{
        CraftedByError, ENTITY_AUTOMATION, ENTITY_ITEMS, ItemPlace, ItemPlaceEditError,
        ItemResource, TransactionError,
    };

    use std::sync::{Arc, mpsc};

    use sunrise_core::geometry::Direction;

    use sunrise_items::items::ItemStack;

    use sunrise_sim::lan::PlayerSender;

    #[test]
    #[ignore = "daily"]
    fn whole_stack_component_edit_preserves_slot_and_refuses_invalid_components() {
        use mod_api::{StackWriteError, Tag};
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[3] = Some(ItemStack::new("minecraft:paper", 16));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let host_before = game.inventory.clone();
            let dropped_before = game.dropped.len();
            let inventory = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .clone()
            };
            let mut expected = inventory(game);
            let name = Tag::String("named paper".into());
            expected[3].as_mut().unwrap().data_mut().custom_name = Some(name.clone());
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let result = game.storage_transaction(|host, tx| {
                Ok::<_, ()>(slot.edit(host, tx, |stack| {
                    stack
                        .apply_components(&[("minecraft:custom_name".into(), name)].into())
                        .unwrap();
                }))
            });
            assert_eq!(result, Ok(Ok(())));
            assert_eq!(game.dropped.len(), dropped_before);
            assert_eq!(
                inventory(game),
                expected,
                "all 16 named paper stay in slot 3"
            );
            assert_eq!(game.inventory, host_before);

            let invalid_consume = Tag::compound([(
                "on_consume_effects",
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("bad", Tag::Double(f64::NAN.to_bits())),
                ])]),
            )]);
            for (id, invalid, refusal) in [
                (
                    "minecraft:max_stack_size",
                    Tag::Int(1),
                    StackWriteError::CountExceeded { maximum: 1 },
                ),
                (
                    "minecraft:consumable",
                    invalid_consume,
                    StackWriteError::InvalidEffectPayload,
                ),
            ] {
                // The parent commits even when the edit is refused; neither component sticks.
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        Ok::<_, ()>(slot.edit(host, tx, |stack| {
                            stack
                                .apply_components(
                                    &[
                                        (
                                            "minecraft:custom_name".into(),
                                            Tag::String("refused rename".into()),
                                        ),
                                        (id.into(), invalid),
                                    ]
                                    .into(),
                                )
                                .unwrap();
                        }))
                    }),
                    Ok(Err(ItemPlaceEditError::Stack(refusal)))
                );
                assert_eq!(inventory(game), expected);
                assert_eq!(game.inventory, host_before);
            }

            let empty = ItemPlace::PlayerSlot {
                player: 123,
                slot: 4,
            };
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    Ok::<_, ()>(empty.edit(host, tx, |_| panic!("empty place ran editor")))
                }),
                Ok(Err(ItemPlaceEditError::NotHeld))
            );
            assert_eq!(inventory(game), expected);
            assert_eq!(game.dropped.len(), dropped_before);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn whole_stack_component_edit_rolls_back_with_its_transaction() {
        use mod_api::{Hand, Tag};
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.selected = 3;
            player.inventory[3] = Some(ItemStack::new("minecraft:paper", 16));
            player.inventory[1] = Some(ItemStack::new("minecraft:paper", 16));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let inventory = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .clone()
            };
            let before = inventory(game);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let mut named = ItemStack::new("minecraft:paper", 1);
            named.data_mut().custom_name = Some(Tag::String("copied name".into()));
            let source = named.try_into_api().unwrap();
            let mut expected = before.clone();
            expected[3].as_mut().unwrap().data_mut().custom_name =
                Some(Tag::String("copied name".into()));
            let expected_stack = expected[3].as_ref().unwrap().try_into_api().unwrap();
            let original_stack = before[3].as_ref().unwrap().try_into_api().unwrap();
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    assert!(
                        slot.edit(host, tx, |stack| {
                            stack.copy_component_from(&source, "minecraft:custom_name");
                        })
                        .is_ok()
                    );
                    assert_eq!(
                        slot.with_stack(host, |stack| stack.project()).flatten(),
                        Some(expected_stack.clone())
                    );
                    Err::<(), ()>(())
                }),
                Err(())
            );
            assert_eq!(inventory(game), before);

            // From a different guest's turn, a hand still names the parked guest's
            // selected slot. Neither editing it nor rolling it back needs a body.
            let mut actor = guest(2);
            actor.selected = 1;
            actor.inventory[1] = Some(ItemStack::new("minecraft:paper", 16));
            game.server.lan.as_mut().unwrap().hub.guests.push(actor);
            let hand = ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            };
            let has_body = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .motion
                    .is_some()
            };
            game.as_guest(2, |game| {
                let acting_before = game.inventory.clone();
                assert!(!has_body(game));
                assert_eq!(
                    game.storage_transaction(|game, _| {
                        let child = game.storage_transaction(|game, tx| {
                            hand.edit(game, tx, |stack| {
                                stack.copy_component_from(&source, "minecraft:custom_name");
                            })
                            .unwrap();
                            assert_eq!(inventory(game), expected);
                            assert_eq!(game.inventory, acting_before);
                            assert!(!has_body(game));
                            Err::<(), ()>(())
                        });
                        assert_eq!(child, Err(()));
                        assert_eq!(
                            hand.with_stack(game, |stack| stack.project()).flatten(),
                            Some(original_stack)
                        );
                        assert_eq!(inventory(game), before);
                        assert_eq!(game.inventory, acting_before);
                        assert!(!has_body(game));
                        Ok::<(), ()>(())
                    }),
                    Ok(())
                );
            })
            .unwrap();
            assert_eq!(inventory(game), before);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_bucket_fills_and_empties_whole_through_its_place_and_splits_off_its_stack() {
        use mod_api::{BlockUpdateFlags, FLUIDS, ITEM_FLUIDS, resource_util::move_resources};
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let cauldron = |game: &sunrise_gameplay::game::ServerGame<'_>, n: u8| {
                let registry = &game.pack.assets.registry;
                if n == 0 {
                    return registry.default_state("minecraft:cauldron").unwrap();
                }
                let state = sunrise_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".to_owned(),
                    properties: [("level".to_owned(), n.to_string())].into_iter().collect(),
                };
                registry.resolve(&state).unwrap()
            };
            let set = |game: &mut sunrise_gameplay::game::ServerGame<'_>, pos, n| {
                let state = cauldron(game, n);
                let flags = BlockUpdateFlags::NEIGHBORS;
                assert_eq!(game.level_set_block(level, pos, state, flags), Ok(true));
            };
            let (a, b) = ([0, 64, 0], [2, 64, 0]);
            set(game, a, 3);
            set(game, b, 2);
            let mut player = guest(1);
            player.inventory[0] = Some(ItemStack::new("minecraft:bucket", 3));
            player.inventory[5] = Some(ItemStack::new("minecraft:lava_bucket", 1));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let slot = |slot| ItemPlace::PlayerSlot { player: 123, slot };
            // Up to a bucket from the cauldron at `at` into the bucket in `slot`, or back.
            let moved = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                         at,
                         slot: &ItemPlace,
                         into_bucket| {
                let host: &mut dyn ServerHost = game;
                let block = level.capability(host, &FLUIDS, at, None).unwrap();
                let bucket = slot.capability(host, &ITEM_FLUIDS).unwrap();
                let (from, to) = match into_bucket {
                    true => (&block, &bucket),
                    false => (&bucket, &block),
                };
                host.transaction(|host, tx| {
                    move_resources(host, from, to, |_| true, 1000, Some(tx))
                })
            };
            let inventory = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .clone()
            };
            let block = |game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]| {
                game.world.block(pos[0], pos[1], pos[2])
            };
            // A full cauldron fills one of the three buckets, which splits off.
            assert_eq!(moved(game, a, &slot(0), true), Ok(1000));
            assert_eq!(block(game, a), cauldron(game, 0));
            let held = inventory(game);
            assert_eq!(held[0], Some(ItemStack::new("minecraft:bucket", 2)));
            let water = ItemStack::new("minecraft:water_bucket", 1);
            let filled = held
                .iter()
                .position(|s| s.as_ref() == Some(&water))
                .unwrap();
            // Two thirds of a bucket fill none: a bucket takes a whole one or nothing.
            assert_eq!(moved(game, b, &slot(0), true), Ok(0));
            assert_eq!(block(game, b), cauldron(game, 2));
            assert_eq!(inventory(game), held);
            // The water bucket empties into the empty cauldron and is a bucket again.
            let filled = slot(i32::try_from(filled).unwrap());
            assert_eq!(moved(game, a, &filled, false), Ok(1000));
            assert_eq!(block(game, a), cauldron(game, 3));
            assert_eq!(
                filled.with_stack(game, |stack| stack.project()).flatten(),
                Some(
                    ItemStack::new("minecraft:bucket", 1)
                        .try_into_api()
                        .unwrap()
                )
            );
            // Lava doesn't go into a water cauldron, and the lava bucket stays full.
            assert_eq!(moved(game, a, &slot(5), false), Ok(0));
            let lava = Some(ItemStack::new("minecraft:lava_bucket", 1));
            assert_eq!(inventory(game)[5], lava);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_guests_place_is_written_as_theirs_and_an_abort_puts_it_back() {
        with_game(|game| {
            open_to_lan(game);
            let mut guest = guest(1);
            guest.inventory[0] = Some(ItemStack::new("minecraft:bucket", 3));
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let host_before = game.inventory.clone();
            let guest_inventory = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .clone()
            };
            let before = guest_inventory(game);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 0,
            };
            let water = ItemResource::of(
                &ItemStack::new("minecraft:water_bucket", 1)
                    .try_into_api()
                    .unwrap(),
            );

            // Aborted, the guest's inventory is as it was, and the host's was never touched.
            let host: &mut dyn ServerHost = game;
            let _ = host.transaction(|host, tx| {
                assert_eq!(slot.exchange(host, tx, &water, 1), Ok(1));
                assert_eq!(slot.amount(host), 2);
                Err::<(), ()>(())
            });
            assert_eq!(guest_inventory(game), before);
            assert_eq!(game.inventory, host_before);

            // Committed, the guest keeps two buckets and gets the filled one.
            let host: &mut dyn ServerHost = game;
            let _ = host.transaction(|host, tx| slot.exchange(host, tx, &water, 1));
            let after = guest_inventory(game);
            assert_eq!(after[0], Some(ItemStack::new("minecraft:bucket", 2)));
            let filled = after.iter().flatten();
            assert_eq!(
                filled
                    .filter(|s| &*s.item == "minecraft:water_bucket")
                    .count(),
                1
            );
            assert_eq!(game.inventory, host_before);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn an_armour_slot_holds_one() {
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let pumpkin = ItemResource::of(
                &ItemStack::new("minecraft:carved_pumpkin", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let wrote = host.transaction(|host, tx| items.insert_at(host, tx, 39, &pumpkin, 64));
            assert_eq!(wrote, Ok(1));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn worn_component_edit_is_quiet_and_reaches_tracking() {
        use mod_api::{EquipmentSlot, Tag};
        use sunrise_items::item_data::trim;
        use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
        use sunrise_sim::{lan::Out, mobs::gear};

        with_resident_game(|game| {
            open_to_lan(game);
            let (tx, rx) = mpsc::channel();
            let mut viewer = guest(1);
            viewer.sender = PlayerSender::new(tx, Arc::default());
            viewer.position = [0.5, 64.0, 1.5];
            viewer.dimension = "minecraft:overworld".into();
            viewer.loaded = true;
            viewer.load_timeout = 0;
            viewer.sent_chunks.insert((0, 0));
            game.server.lan.as_mut().unwrap().hub.guests.push(viewer);

            // With no tickets every chunk is entity-ticking; the zombie must lie outside that
            // range for the tracker to describe it idle (`ChunkMap.tick`).
            game.server
                .tickets
                .0
                .update(Some(sunrise_sim::ticket_levels::Sources {
                    players: vec![(40, 40)],
                    view_distance: 2,
                    simulation_distance: 2,
                    ..Default::default()
                }));
            let id = zombie(game);
            let living = game.living(game.entity(id).unwrap()).unwrap();
            let slot = EquipmentSlot::Chest;
            let mut armor = ItemStack::new("minecraft:diamond_chestplate", 1);
            armor
                .data_mut()
                .enchantments
                .push(("minecraft:protection".into(), 1));
            armor.data_mut().trim = Some(("minecraft:iron".into(), "minecraft:coast".into()));
            assert!(game.living_set_equipment(living, slot, Some(armor)));
            // The separate gear data can differ from the resident stack.
            let worn = game.mobs[0].extra.gear.as_deref_mut().unwrap();
            worn.slots[gear::CHEST][0].1 = 3;
            worn.trims[gear::CHEST] = Some(("minecraft:gold".into(), "minecraft:sentry".into()));
            game.mobs[0].first_tick = false;
            // Track once to pair, again to cache a full idle state, then confirm it can go bare.
            for _ in 0..3 {
                game.server.level_pass.packets_drained = true;
                game.mobs[0].needs_sync = false;
                game.tick_lan(false);
            }
            let host = game.server.lan.as_ref().unwrap();
            assert!(host.hub.guests[0].tracker.is_tracking(id as i32));
            assert!(
                host.states
                    .as_slice()
                    .iter()
                    .any(|state| state.id == id as i32 && state.bare)
            );
            rx.try_iter().for_each(drop);

            game.start_audio_capture();
            let place = ItemPlace::Equipment { entity: id, slot };
            sunrise_gameplay::game::server_side::dispense_fixture::start_recording();
            game.storage_transaction(|host, tx| {
                place.edit(host, tx, |stack| {
                    let original = stack.original().project().unwrap();
                    stack.copy_component_from(&original, "minecraft:enchantments");
                    stack.copy_component_from(&original, "minecraft:trim");
                    stack.set_damage(7);
                })
            })
            .unwrap();
            assert!(game.server.pending_sounds.is_empty());
            assert!(
                sunrise_gameplay::game::server_side::dispense_fixture::stop_recording().is_empty()
            );

            let tracked = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.server.level_pass.packets_drained = true;
                game.tick_lan(false);
                let hub = &game.server.lan.as_ref().unwrap().hub;
                rx.try_iter()
                    .flat_map(|message| match message {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(reason) => {
                            panic!("tracking viewer disconnected: {reason:?}")
                        }
                    })
                    .find_map(|body| {
                        let mut reader = Reader::new(&body);
                        let packet = reader.varint().ok()?;
                        let Clientbound::SetEquipment(equipment) =
                            Clientbound::decode(packet, reader.rest()).ok()?
                        else {
                            return None;
                        };
                        if equipment.id != id as i32 {
                            return None;
                        }
                        let (_, stack) = equipment
                            .slots
                            .into_iter()
                            .find(|(at, _)| *at == slot as u8)?;
                        Some(hub.stack_from_network(&stack?))
                    })
                    .expect("the existing viewer receives the idle mob's equipment edit")
            };
            let received = tracked(game);
            assert_eq!(received.item.as_ref(), "minecraft:diamond_chestplate");
            assert_eq!(received.count, 1);
            assert_eq!(received.damage, 7);
            assert_eq!(received.tag_enchantment_level("minecraft:protection"), 3);
            assert_eq!(
                received.data().unwrap().trim.as_ref().and_then(trim::named),
                Some(("minecraft:gold", "minecraft:sentry"))
            );
            let resident = game.mobs[0].extra.equipment.get(slot as usize).unwrap();
            assert_eq!(resident.tag_enchantment_level("minecraft:protection"), 1);
            assert_eq!(
                resident.data().unwrap().trim.as_ref().and_then(trim::named),
                Some(("minecraft:iron", "minecraft:coast"))
            );

            game.storage_transaction(|host, tx| {
                place.edit(host, tx, |stack| {
                    stack
                        .apply_components(
                            &[(
                                "!minecraft:enchantments".into(),
                                Tag::Compound(Default::default()),
                            )]
                            .into(),
                        )
                        .unwrap();
                })
            })
            .unwrap();
            let resident = game.mobs[0].extra.equipment.get(slot as usize).unwrap();
            assert!(
                resident
                    .data()
                    .unwrap()
                    .other("!minecraft:enchantments")
                    .is_some()
            );
            let received = tracked(game);
            assert_eq!(received.damage, 7);
            assert!(received.enchantments().is_empty());
            assert_eq!(
                received.data().unwrap().trim.as_ref().and_then(trim::named),
                Some(("minecraft:gold", "minecraft:sentry"))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn virtual_wrappers_refuse_a_whole_stack_edit() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let stick = ItemStack::new("minecraft:stick", 32);
            let cursor_stack = ItemStack::new("minecraft:paper", 1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.game_mode = 1;
                    guest.inventory[0] = Some(stick.clone());
                    guest.carried = Some(cursor_stack.clone());
                });
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let hand = ItemPlace::PlayerHand {
                player: player.uuid(),
                hand: mod_api::Hand::Main,
            };
            for place in [
                ItemPlace::Strict(Box::new(ItemPlace::InfiniteMaterials {
                    player: player.uuid(),
                    inner: Box::new(hand.clone()),
                })),
                ItemPlace::OneByOne(Box::new(hand.clone())),
            ] {
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        place.edit(host, tx, |_| panic!("a refused place ran its editor"))
                    }),
                    Err(ItemPlaceEditError::UnsupportedPlace)
                );
            }
            for place in [&hand, &menu.cursor()] {
                let mut edited = false;
                let mismatch = game.storage_transaction(|game, outer| {
                    game.storage_transaction(|game, _| place.edit(game, outer, |_| edited = true))
                });
                assert!(!edited);
                assert_eq!(mismatch, Err(ItemPlaceEditError::InvalidContext));
            }
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.inventory[0].as_ref(), Some(&stick));
            assert_eq!(guest.carried.as_ref(), Some(&cursor_stack));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_cursor_whole_stack_edit_validates_and_rolls_back() {
        use mod_api::{MenuReplace, StackWriteError, Tag};

        with_game(|game| {
            open_to_lan(game);
            let mut player_guest = guest(1);
            player_guest.carried = Some(ItemStack::new("minecraft:paper", 16));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(player_guest);
            let player = game.player(123).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let cursor = menu.cursor();
            let strict_cursor = ItemPlace::Strict(Box::new(cursor.clone()));

            // Strict preserves resolved whole-stack cursor editing without changing item or count.
            let parked_name = Tag::String("cursor paper".into());
            let mut expected = ItemStack::new("minecraft:paper", 16);
            expected.data_mut().custom_name = Some(parked_name.clone());
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    strict_cursor.edit(host, tx, |stack| {
                        assert_eq!(
                            mod_api::__private::item_key_id(stack.item())
                                .unwrap()
                                .to_string(),
                            "minecraft:paper"
                        );
                        assert_eq!(stack.count(), 16);
                        stack
                            .apply_components(
                                &[("minecraft:custom_name".into(), parked_name.clone())].into(),
                            )
                            .unwrap();
                    })
                }),
                Ok(())
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref(),
                Some(&expected)
            );

            // Invalid component write refuses cleanly without applying partial edits.
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    Ok::<_, ()>(cursor.edit(host, tx, |stack| {
                        stack
                            .apply_components(
                                &[
                                    (
                                        "minecraft:custom_name".into(),
                                        Tag::String("refused rename".into()),
                                    ),
                                    ("minecraft:max_stack_size".into(), Tag::Int(1)),
                                ]
                                .into(),
                            )
                            .unwrap();
                    }))
                }),
                Ok(Err(ItemPlaceEditError::Stack(
                    StackWriteError::CountExceeded { maximum: 1 }
                )))
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref(),
                Some(&expected)
            );

            // Aborting the transaction restores the parked cursor stack.
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    cursor
                        .edit(host, tx, |stack| {
                            stack
                                .apply_components(
                                    &[(
                                        "minecraft:custom_name".into(),
                                        Tag::String("aborted rename".into()),
                                    )]
                                    .into(),
                                )
                                .unwrap();
                        })
                        .unwrap();
                    Err::<(), ()>(())
                }),
                Err(())
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref(),
                Some(&expected)
            );

            // Acting guest cursor edit rolls back on abort and commits in place.
            let acting_name = Tag::String("acting cursor".into());
            let mut acting_expected = expected.clone();
            acting_expected.data_mut().custom_name = Some(acting_name.clone());
            game.as_guest(1, |game| {
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        cursor
                            .edit(host, tx, |stack| {
                                stack
                                    .apply_components(
                                        &[(
                                            "minecraft:custom_name".into(),
                                            Tag::String("acting abort".into()),
                                        )]
                                        .into(),
                                    )
                                    .unwrap();
                            })
                            .unwrap();
                        Err::<(), ()>(())
                    }),
                    Err(())
                );
                assert_eq!(game.carried.as_ref(), Some(&expected));

                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        cursor.edit(host, tx, |stack| {
                            stack
                                .apply_components(
                                    &[("minecraft:custom_name".into(), acting_name.clone())].into(),
                                )
                                .unwrap();
                        })
                    }),
                    Ok(())
                );
                assert_eq!(game.carried.as_ref(), Some(&acting_expected));
            })
            .unwrap();
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref(),
                Some(&acting_expected)
            );

            // Empty cursor refuses with NotHeld before calling the editor.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = None);
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    cursor.edit(host, tx, |_| panic!("empty cursor ran editor"))
                }),
                Err(ItemPlaceEditError::NotHeld)
            );

            // Stale or closed menu handles refuse with NotHeld before calling the editor.
            let player = game.player(123).unwrap();
            let open_menu = player
                .open_chest_menu(
                    game,
                    1,
                    mod_api::Text::literal("box"),
                    Vec::new(),
                    MenuReplace::CloseFirst,
                )
                .unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(ItemStack::new("minecraft:paper", 2));
            });
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    cursor.edit(host, tx, |_| panic!("stale cursor ran editor"))
                }),
                Err(ItemPlaceEditError::NotHeld)
            );
            assert!(open_menu.close_container(game));
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(ItemStack::new("minecraft:paper", 2));
            });
            assert_eq!(
                game.storage_transaction(|host, tx| {
                    open_menu
                        .cursor()
                        .edit(host, tx, |_| panic!("closed menu cursor ran editor"))
                }),
                Err(ItemPlaceEditError::NotHeld)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_cursor_edit_syncs_set_cursor_item_once() {
        use mod_api::{MenuReplace, Tag};
        use sunrise_protocol::{
            buf::Reader,
            item::{Component, component},
            packets::play::Clientbound as C,
        };
        use sunrise_sim::lan::Out;

        fn cursor_packets(rx: &mpsc::Receiver<Out>) -> Vec<Option<sunrise_protocol::item::Stack>> {
            rx.try_iter()
                .flat_map(|out| match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    _ => Vec::new(),
                })
                .filter_map(|body| {
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match C::decode(id, reader.rest()).unwrap() {
                        C::SetCursorItem(packet) => Some(packet.item),
                        _ => None,
                    }
                })
                .collect()
        }

        with_game(|game| {
            open_to_lan(game);
            let (tx, rx) = mpsc::channel();
            let mut player_guest = guest(1);
            player_guest.sender = PlayerSender::new(tx, Arc::default());
            player_guest.carried = Some(ItemStack::new("minecraft:stick", 1));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(player_guest);

            // Sync initial inventory + cursor; subsequent tick sync sends no SetCursorItem.
            game.server.lan.as_mut().unwrap().hub.send_inventory(0);
            let _ = cursor_packets(&rx);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_packets(&rx).is_empty());

            let direct_instrument = Tag::compound([
                (
                    "sound_event",
                    Tag::compound([("sound_id", Tag::String("test:horn".into()))]),
                ),
                ("use_duration", Tag::Float(1.5_f32.to_bits())),
                ("range", Tag::Float(32.0_f32.to_bits())),
                ("durability_damage", Tag::Int(1)),
                ("description", Tag::String("Test Horn".into())),
            ]);
            // Parked cursor edit syncs once via sync_player_state (sync_guest_after_tick).
            let player = game.player(123).unwrap();
            let inv_menu = game.player_container_menu(player).unwrap();
            game.storage_transaction(|host, tx| {
                inv_menu.cursor().edit(host, tx, |stack| {
                    assert_eq!(stack.damage_component(), None);
                    stack
                        .apply_components(
                            &[
                                ("minecraft:max_stack_size".into(), Tag::Int(1)),
                                ("minecraft:max_damage".into(), Tag::Short(20)),
                                ("minecraft:damage".into(), Tag::Int(0)),
                                (
                                    "minecraft:consumable".into(),
                                    Tag::compound([
                                        ("consume_seconds", Tag::Float(0.75_f32.to_bits())),
                                        ("animation", Tag::String("bow".into())),
                                        ("has_consume_particles", Tag::Byte(0)),
                                    ]),
                                ),
                                ("minecraft:instrument".into(), direct_instrument.clone()),
                                (
                                    "minecraft:death_protection".into(),
                                    Tag::compound([("death_effects", Tag::List(Vec::new()))]),
                                ),
                                (
                                    "minecraft:villager_food".into(),
                                    Tag::compound([("nutrition", Tag::Int(7))]),
                                ),
                                (
                                    "minecraft:potion_contents".into(),
                                    Tag::compound([("custom_color", Tag::Int(0x123456))]),
                                ),
                            ]
                            .into(),
                        )
                        .unwrap();
                })
            })
            .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            let updates = cursor_packets(&rx);
            let [Some(parked)] = updates.as_slice() else {
                panic!("expected one carried stack update, got {updates:?}");
            };
            assert_eq!(parked.count, 1);
            assert_eq!(
                parked.component(component::MAX_STACK_SIZE),
                Some(&Component::Int(1))
            );
            assert_eq!(
                parked.component(component::MAX_DAMAGE),
                Some(&Component::Int(20))
            );
            assert_eq!(
                parked.component(component::DAMAGE),
                Some(&Component::Int(0))
            );
            let client = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .stack_from_network(parked);
            let consumable = client
                .consumable(None)
                .expect("cursor's consumable override");
            assert_eq!(consumable.seconds.to_bits(), 0.75_f32.to_bits());
            assert!(!consumable.particles);
            assert_eq!(
                consumable.animation,
                sunrise_core::use_animation::UseAnimation::Bow
            );
            let instrument = client
                .data()
                .unwrap()
                .other("minecraft:instrument")
                .unwrap();
            assert!(matches!(
                parked.component(component::INSTRUMENT),
                Some(Component::Raw(_))
            ));
            for name in ["sound_event", "use_duration", "range", "durability_damage"] {
                assert_eq!(instrument.get(name), direct_instrument.get(name), "{name}");
            }
            assert_eq!(
                sunrise_game_data::component::name_text(instrument.get("description").unwrap())
                    .plain(),
                "Test Horn"
            );
            assert!(client.death_protection(None).is_some_and(<[_]>::is_empty));
            assert_eq!(client.villager_food(None), Some(7));
            assert_eq!(
                parked.component(component::POTION_CONTENTS),
                Some(&Component::Potion {
                    potion: None,
                    color: Some(0x123456),
                    // No custom effects and no custom name.
                    rest: Vec::new(),
                })
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_packets(&rx).is_empty());

            // A resident component whose registry reference cannot be represented for
            // this connection must not block an independent cursor edit or cause a retry.
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.update(0, |guest| {
                let stack = guest.carried.as_mut().unwrap();
                stack.data_mut().set_other(
                    "minecraft:jukebox_playable",
                    Some(Tag::String("test:missing_song".into())),
                );
                stack.damage = 7;
            });
            hub.sync_guest_after_tick(1);
            let updates = cursor_packets(&rx);
            let [Some(edited)] = updates.as_slice() else {
                panic!("expected one cursor update despite the unresolved song, got {updates:?}");
            };
            assert_eq!(
                edited.component(component::DAMAGE),
                Some(&Component::Int(7))
            );
            hub.sync_guest_after_tick(1);
            assert!(cursor_packets(&rx).is_empty());

            // Open a container menu, populate and snapshot its cursor, then edit while acting.
            let player = game.player(123).unwrap();
            let open_menu = player
                .open_chest_menu(
                    game,
                    1,
                    mod_api::Text::literal("box"),
                    Vec::new(),
                    MenuReplace::CloseFirst,
                )
                .unwrap();
            let window = open_menu.container_id(game).unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(ItemStack::new("minecraft:paper", 4));
            });
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .resend_menu(1, window, &game.common.world);
            let _ = cursor_packets(&rx);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_containers(&game.common.world);
            assert!(cursor_packets(&rx).is_empty());

            let acting_name = Tag::String("synced acting cursor".into());
            let mut acting_expected = ItemStack::new("minecraft:paper", 4);
            acting_expected.data_mut().custom_name = Some(acting_name.clone());
            game.as_guest(1, |game| {
                game.storage_transaction(|host, tx| {
                    open_menu.cursor().edit(host, tx, |stack| {
                        stack
                            .apply_components(
                                &[("minecraft:custom_name".into(), acting_name)].into(),
                            )
                            .unwrap();
                    })
                })
                .unwrap();
            })
            .unwrap();
            let acting_net = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .stack(&Some(acting_expected));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_containers(&game.common.world);
            assert_eq!(cursor_packets(&rx), vec![acting_net]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_containers(&game.common.world);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_packets(&rx).is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn cursor_edit_interleaved_with_stale_component_hash_click_still_syncs() {
        use mod_api::{BlockUpdateFlags, Tag};
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::{
            buf::Reader,
            item::Hashed,
            packets::play::{Clientbound as C, ContainerClick, Serverbound as S},
            registry::SyncedNames,
        };
        use sunrise_sim::lan::Out;

        fn drain_packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
            rx.try_iter()
                .flat_map(|out| match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    _ => Vec::new(),
                })
                .map(|body| {
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    C::decode(id, reader.rest()).unwrap()
                })
                .collect()
        }

        fn cursor_updates(rx: &mpsc::Receiver<Out>) -> Vec<Option<sunrise_protocol::item::Stack>> {
            drain_packets(rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::SetCursorItem(update) => Some(update.item),
                    _ => None,
                })
                .collect()
        }

        fn hash_stack(
            game: &mut sunrise_gameplay::game::ServerGame<'_>,
            stack: &Option<ItemStack>,
        ) -> Option<Hashed> {
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            let net = hub.stack(stack)?;
            let sync = hub.sync_data();
            let names = SyncedNames {
                builtin: &sync.builtin,
                dynamic: &sync.dynamic,
            };
            Some(Hashed::create(&net, &names).unwrap())
        }

        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let table = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafting_table")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 0], table, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );

            let (tx, rx) = mpsc::channel();
            let mut player_guest = guest(1);
            player_guest.position = [1.5, 64.0, 0.5];
            player_guest.sender = PlayerSender::new(tx, Arc::default());
            let plain_paper = Some(ItemStack::new("minecraft:paper", 4));
            player_guest.carried = plain_paper.clone();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .push(player_guest);
            game.server.lan.as_mut().unwrap().hub.send_inventory(0);
            let _ = drain_packets(&rx);

            // Answer-click path: mod renames the carried paper (same item and count), then a
            // click arrives carrying the pre-edit component hash. The server must reject the
            // stale prediction and send SetCursorItem instead of caching the renamed stack.
            let stale_inv_hash = hash_stack(game, &plain_paper);
            let player = game.player(123).unwrap();
            let inv_menu = game.player_container_menu(player).unwrap();
            let renamed_tag = Tag::String("renamed paper".into());
            let mut renamed_paper = ItemStack::new("minecraft:paper", 4);
            renamed_paper.data_mut().custom_name = Some(renamed_tag.clone());
            game.storage_transaction(|host, tx| {
                inv_menu.cursor().edit(host, tx, |stack| {
                    stack
                        .apply_components(&[("minecraft:custom_name".into(), renamed_tag)].into())
                        .unwrap();
                })
            })
            .unwrap();
            let inv_state = game.server.lan.as_ref().unwrap().hub.guests[0].menu_state;
            game.lan_guest_packet(
                1,
                S::ContainerClick(ContainerClick {
                    container: 0,
                    state: inv_state,
                    slot: 0,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: stale_inv_hash,
                }),
            );
            let renamed_net = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .stack(&Some(renamed_paper.clone()));
            assert_eq!(cursor_updates(&rx), vec![renamed_net]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_updates(&rx).is_empty());

            // A follow-up click with the exact current component hash agrees silently.
            let fresh_inv_hash = hash_stack(game, &Some(renamed_paper.clone()));
            let inv_state = game.server.lan.as_ref().unwrap().hub.guests[0].menu_state;
            game.lan_guest_packet(
                1,
                S::ContainerClick(ContainerClick {
                    container: 0,
                    state: inv_state,
                    slot: 0,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: fresh_inv_hash.clone(),
                }),
            );
            assert!(cursor_updates(&rx).is_empty());
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_updates(&rx).is_empty());

            // Crafting-table path: open table (syncing renamed_paper), then change carried's
            // custom_name (same item and count) before a click with the stale component hash.
            game.lan_open_crafting(1, [1, 64, 0]);
            let _ = drain_packets(&rx);
            let table_tag = Tag::String("table renamed paper".into());
            let mut table_paper = ItemStack::new("minecraft:paper", 4);
            table_paper.data_mut().custom_name = Some(table_tag);
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(table_paper.clone());
            });
            let (table_window, table_state) = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .menus
                .get(&1)
                .and_then(|play| play.crafting.table.as_ref())
                .map(|t| (t.0, t.1))
                .unwrap();
            game.lan_guest_packet(
                1,
                S::ContainerClick(ContainerClick {
                    container: table_window,
                    state: table_state,
                    slot: 0,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: fresh_inv_hash,
                }),
            );
            let table_net = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .stack(&Some(table_paper.clone()));
            // Matching state IDs take `handleContainerClick`'s incremental broadcast path.
            // A wrong carried hash corrects the cursor without resending the whole menu.
            let replies = drain_packets(&rx);
            assert!(!replies.iter().any(|packet| matches!(
                packet,
                C::ContainerSetContent(content) if content.container == table_window
            )));
            let corrected_cursor: Vec<_> = replies
                .into_iter()
                .filter_map(|packet| match packet {
                    C::SetCursorItem(update) => Some(update.item),
                    _ => None,
                })
                .collect();
            assert_eq!(corrected_cursor, vec![table_net]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_updates(&rx).is_empty());

            // A crafting click with the exact component hash agrees and sends no resync.
            let fresh_table_hash = hash_stack(game, &Some(table_paper));
            let (table_window, table_state) = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .menus
                .get(&1)
                .and_then(|play| play.crafting.table.as_ref())
                .map(|t| (t.0, t.1))
                .unwrap();
            game.lan_guest_packet(
                1,
                S::ContainerClick(ContainerClick {
                    container: table_window,
                    state: table_state,
                    slot: 0,
                    button: 0,
                    input: 1,
                    changed: Vec::new(),
                    carried: fresh_table_hash,
                }),
            );
            let resyncs: Vec<_> = drain_packets(&rx)
                .into_iter()
                .filter(|packet| matches!(packet, C::SetCursorItem(_) | C::ContainerSetContent(_)))
                .collect();
            assert!(resyncs.is_empty());
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_guest_after_tick(1);
            assert!(cursor_updates(&rx).is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn component_edit_preserves_live_nested_data_and_applies_a_removal() {
        use mod_api::Tag;

        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            let mut map = ItemStack::new("minecraft:filled_map", 1);
            map.data_mut().map_id = Some(27);
            map.data_mut().map_post_processing = Some(1);
            let mut bundle = ItemStack::new("minecraft:bundle", 1);
            let data = bundle.data_mut();
            data.bundle_contents.push(map);
            data.bundle_selected = sunrise_items::item_data::BundleSelected(0);
            data.custom_name = Some(Tag::String("keep my contents".into()));
            player.inventory[3] = Some(bundle);
            let place = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            game.server.lan.as_mut().unwrap().hub.guests.push(player);

            let name = Tag::String("live contents".into());
            game.storage_transaction(|host, tx| {
                place.edit(host, tx, |stack| {
                    let original = stack.original().project().unwrap();
                    stack.copy_component_from(&original, "minecraft:bundle_contents");
                    stack
                        .apply_components(
                            &[
                                ("minecraft:item_name".into(), name.clone()),
                                (
                                    "!minecraft:custom_name".into(),
                                    Tag::Compound(Default::default()),
                                ),
                                (
                                    "minecraft:villager_food".into(),
                                    Tag::compound([("nutrition", Tag::Int(7))]),
                                ),
                            ]
                            .into(),
                        )
                        .unwrap();
                })
            })
            .unwrap();
            let bundle = game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3]
                .as_ref()
                .unwrap();
            assert_eq!(bundle.villager_food(None), Some(7));

            game.storage_transaction(|host, tx| {
                place.edit(host, tx, |stack| {
                    stack
                        .apply_components(
                            &[(
                                "!minecraft:villager_food".into(),
                                Tag::Compound(Default::default()),
                            )]
                            .into(),
                        )
                        .unwrap();
                })
            })
            .unwrap();
            let bundle = game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3]
                .as_ref()
                .unwrap();
            assert_eq!(bundle.villager_food(None), None);
            let data = bundle.data().unwrap();
            assert!(data.other("!minecraft:villager_food").is_some());
            assert!(data.custom_name.is_none());
            assert!(data.other("!minecraft:custom_name").is_some());
            assert_eq!(data.other("minecraft:item_name"), Some(&name));
            assert_eq!(data.bundle_selected.0, 0);
            let map = data.bundle_contents[0].data().unwrap();
            assert_eq!(map.map_id, Some(27));
            assert_eq!(map.map_post_processing, Some(1));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn editing_a_raid_banners_name_preserves_its_other_components() {
        use mod_api::Tag;
        use sunrise_items::item_data::encode_extra_components;

        with_game(|game| {
            open_to_lan(game);
            let banner = sunrise_sim::mobs::raider::ominous_banner();
            let original = encode_extra_components(&banner).unwrap();
            let mut player = guest(1);
            player.inventory[3] = Some(banner);
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let place = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let edit = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id: &str, value: Tag| {
                game.storage_transaction(|host, tx| {
                    place.edit(host, tx, |stack| {
                        stack
                            .apply_components(&[(id.into(), value)].into())
                            .unwrap();
                    })
                })
                .unwrap();
            };
            let banner = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3]
                    .clone()
                    .unwrap()
            };
            edit(
                game,
                "minecraft:custom_name",
                Tag::String("Named Raid Banner".into()),
            );
            let compact = banner(game);
            assert!(compact.data().unwrap().ominous_banner);
            let mut named = sunrise_sim::mobs::raider::ominous_banner();
            named.data_mut().set_other(
                "minecraft:custom_name",
                Some(Tag::String("Named Raid Banner".into())),
            );
            assert!(compact.same_item_same_components(&named));

            let name = Tag::String("Renamed Raid Banner".into());
            edit(game, "minecraft:item_name", name.clone());
            let edited = encode_extra_components(&banner(game)).unwrap();
            assert_eq!(edited.get("minecraft:item_name"), Some(&name));
            for id in [
                "minecraft:banner_patterns",
                "minecraft:rarity",
                "minecraft:tooltip_display",
            ] {
                assert_eq!(
                    edited.get(id).unwrap(),
                    original.get(id).unwrap(),
                    "{id} was not edited"
                );
            }

            let rare = Tag::String("rare".into());
            edit(game, "minecraft:rarity", rare.clone());
            edit(
                game,
                "minecraft:item_name",
                original.get("minecraft:item_name").unwrap().clone(),
            );
            let saved = sunrise_items::item_nbt::item_to_nbt(&banner(game), game.items());
            let loaded = sunrise_items::item_nbt::item_from_nbt(&saved).unwrap();
            for stack in [banner(game), loaded] {
                let components = encode_extra_components(&stack).unwrap();
                assert_eq!(components.get("minecraft:rarity"), Some(&rare));
                for id in [
                    "minecraft:item_name",
                    "minecraft:banner_patterns",
                    "minecraft:tooltip_display",
                ] {
                    assert_eq!(components.get(id), original.get(id), "{id} must survive");
                }
            }
            // Writing the last differing value restores the stock representation,
            // so a resource reconstructed from the same components compares equal.
            let fresh = sunrise_sim::mobs::raider::ominous_banner();
            let prototype = fresh.try_into_api().unwrap();
            game.storage_transaction(|host, tx| {
                place.edit(host, tx, |stack| {
                    for id in ["minecraft:rarity", "minecraft:custom_name"] {
                        stack.copy_component_from(&prototype, id);
                    }
                })
            })
            .unwrap();
            assert!(banner(game).same_item_same_components(&fresh));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn nested_equipment_rollback_restores_the_live_modifiers_not_the_quiet_stack() {
        use mod_api::{EquipmentSlot, Tag};
        use sunrise_sim::mobs::gear;

        with_resident_game(|game| {
            let id = zombie(game);
            let living = game.living(game.entity(id).unwrap()).unwrap();
            let slot = EquipmentSlot::Chest;
            let place = ItemPlace::Equipment { entity: id, slot };
            assert!(game.living_set_equipment(
                living,
                slot,
                Some(ItemStack::new("minecraft:diamond_chestplate", 1))
            ));
            // A mob can have gear that the raw stack does not carry at all.
            gear::upgrade(&mut game.mobs[0], gear::CHEST, "minecraft:protection", 2);
            let armor = |host: &dyn mod_api::ServerHost| {
                living.attribute_value(host, "minecraft:armor").unwrap()
            };
            let before = armor(game);
            assert!(before >= 8.0);
            for (commit_child, gravity) in [(false, 0.1), (true, 0.2)] {
                let rolled_back = game.storage_transaction(|host, tx| {
                    place
                        .edit(host, tx, |stack| {
                            stack
                                .apply_components(
                                    &[(
                                        "!minecraft:attribute_modifiers".into(),
                                        Tag::compound(Vec::<(&str, Tag)>::new()),
                                    )]
                                    .into(),
                                )
                                .unwrap();
                        })
                        .unwrap();
                    assert_eq!(
                        armor(host),
                        before,
                        "quiet edit must not recollect the worn stack"
                    );
                    let taken = place.resource(host).unwrap();
                    let child = host.transaction(|host, tx| {
                        assert_eq!(place.extract(host, tx, &taken, 1), Ok(1));
                        assert!(armor(host) < before);
                        assert!(living.set_attribute_base(host, "minecraft:gravity", gravity));
                        // This disjoint write invalidates the live map. Rollback must use
                        // the updated seed and restore armor without asking the item again.
                        assert!(living.set_attribute_base(host, "minecraft:max_health", 30.0));
                        commit_child.then_some(()).ok_or(())
                    });
                    assert_eq!(child.is_ok(), commit_child);
                    assert_eq!(
                        living.attribute_base(host, "minecraft:gravity"),
                        Some(gravity)
                    );
                    if !commit_child {
                        assert_eq!(
                            armor(host),
                            before,
                            "child abort restores the still-active modifiers"
                        );
                        assert_eq!(place.resource(host), Some(taken));
                    }
                    Err::<(), ()>(())
                });
                assert_eq!(rolled_back, Err(()));
                assert_eq!(
                    living.attribute_base(game, "minecraft:gravity"),
                    Some(gravity)
                );
                assert_eq!(
                    living.attribute_base(game, "minecraft:max_health"),
                    Some(30.0)
                );
                assert_eq!(
                    armor(game),
                    before,
                    "parent abort restores the original modifiers"
                );
                let mob = &game.mobs[0];
                let stack = mob.extra.equipment.get(slot as usize).unwrap();
                assert_eq!(stack.item.as_ref(), "minecraft:diamond_chestplate");
                assert!(stack.data().is_none(), "rollback restores the raw stack");
                assert_eq!(gear::level(mob, gear::CHEST, "minecraft:protection"), 2);
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn equipment_rollback_undoes_a_rebuild_after_a_quiet_edit() {
        use mod_api::{EquipmentSlot, Tag};

        with_resident_game(|game| {
            let entity = zombie(game);
            let living = game.living(game.entity(entity).unwrap()).unwrap();
            let slot = EquipmentSlot::Chest;
            let place = ItemPlace::Equipment { entity, slot };
            assert!(game.living_set_equipment(
                living,
                slot,
                Some(ItemStack::new("minecraft:diamond_chestplate", 1))
            ));
            let before = place.resource(game).unwrap();
            let armor =
                |host: &dyn ServerHost| living.attribute_value(host, "minecraft:armor").unwrap();
            let worn_armor = armor(game);
            assert!(worn_armor >= 8.0);
            let result = game.storage_transaction(|host, tx| {
                place
                    .edit(host, tx, |stack| {
                        stack
                            .apply_components(
                                &[(
                                    "!minecraft:attribute_modifiers".into(),
                                    Tag::Compound(Default::default()),
                                )]
                                .into(),
                            )
                            .unwrap();
                    })
                    .unwrap();
                assert_eq!(
                    armor(host),
                    worn_armor,
                    "the quiet edit does not refresh armor"
                );
                assert!(living.set_attribute_base(host, "minecraft:max_health", 32.0));
                assert!(
                    armor(host) < worn_armor,
                    "the rebuild sees the tentative edit"
                );
                Err::<(), ()>(())
            });
            assert_eq!(result, Err(()));
            assert_eq!(place.resource(game), Some(before));
            assert_eq!(
                armor(game),
                worn_armor,
                "rollback restores the pre-rebuild live modifiers"
            );
            assert_eq!(
                living.attribute_base(game, "minecraft:max_health"),
                Some(32.0)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn equipment_rollback_keeps_a_later_permanent_modifier_with_the_same_id() {
        use mod_api::{
            AttributeModifier, AttributeOperation, EquipmentSlot, Identifier, ModifierLifetime, Tag,
        };

        with_resident_game(|game| {
            let entity = zombie(game);
            let living = game.living(game.entity(entity).unwrap()).unwrap();
            let slot = EquipmentSlot::Chest;
            let place = ItemPlace::Equipment { entity, slot };
            let attribute = "minecraft:gravity";
            let modifier = "test:bonus";
            let gravity = |host: &dyn ServerHost| living.attribute_value(host, attribute).unwrap();
            assert!(living.set_attribute_base(game, attribute, 0.125));
            let mut armor = ItemStack::new("minecraft:diamond_chestplate", 1);
            armor.data_mut().set_other(
                "minecraft:attribute_modifiers",
                Some(Tag::List(vec![Tag::compound([
                    ("type", Tag::String(attribute.into())),
                    ("id", Tag::String(modifier.into())),
                    ("amount", Tag::Double(0.25f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                    ("slot", Tag::String("chest".into())),
                ])])),
            );
            assert!(game.living_set_equipment(living, slot, Some(armor)));
            assert_eq!(gravity(game), 0.375);
            let resource = place.resource(game).unwrap();
            game.storage_transaction(|host, _| {
                let child = host.transaction(|host, tx| {
                    assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                    assert_eq!(gravity(host), 0.125);
                    assert!(living.add_attribute_modifier(
                        host,
                        attribute,
                        AttributeModifier::new(
                            Identifier::parse(modifier).unwrap(),
                            0.5,
                            AttributeOperation::AddValue
                        ),
                        ModifierLifetime::Permanent,
                    ));
                    Err::<(), ()>(())
                });
                assert_eq!(child, Err(TransactionError::Aborted(())));
                assert_eq!(place.resource(host), Some(resource));
                assert_eq!(gravity(host), 0.625);
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(gravity(game), 0.625);
            let entry = saved_mob_attribute(game, entity, attribute);
            assert_eq!(
                entry.get("modifiers"),
                Some(&Tag::List(vec![Tag::compound([
                    ("id", Tag::String(modifier.into())),
                    ("amount", Tag::Double(0.5f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                ])]))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_same_id_gear_refresh_only_keeps_its_overwrite_on_commit() {
        use mod_api::{
            AttributeModifier, AttributeOperation, EquipmentSlot, Identifier, ModifierLifetime, Tag,
        };

        for commit in [false, true] {
            with_resident_game(|game| {
                let entity = zombie(game);
                let living = game.living(game.entity(entity).unwrap()).unwrap();
                let slot = EquipmentSlot::Chest;
                let place = ItemPlace::Equipment { entity, slot };
                let attribute = "minecraft:max_health";
                let modifier = "test:bonus";
                assert!(living.set_attribute_base(game, attribute, 20.0));
                assert!(game.living_set_equipment(
                    living,
                    slot,
                    Some(ItemStack::new("minecraft:diamond_chestplate", 1))
                ));
                let before = place.resource(game).unwrap();
                let mut after = None;
                let result = game.storage_transaction(|host, tx| {
                    place
                        .edit(host, tx, |stack| {
                            let rows = Tag::List(vec![Tag::compound([
                                ("type", Tag::String(attribute.into())),
                                ("id", Tag::String(modifier.into())),
                                ("amount", Tag::Double(3.0f64.to_bits())),
                                ("operation", Tag::String("add_value".into())),
                                ("slot", Tag::String("chest".into())),
                            ])]);
                            stack
                                .apply_components(
                                    &[("minecraft:attribute_modifiers".into(), rows)].into(),
                                )
                                .unwrap();
                        })
                        .unwrap();
                    assert_eq!(
                        living.attribute_value(host, attribute),
                        Some(20.0),
                        "the edit is quiet"
                    );
                    after = place.resource(host);
                    assert_ne!(after.as_ref(), Some(&before));
                    assert!(living.add_attribute_modifier(
                        host,
                        attribute,
                        AttributeModifier::new(
                            Identifier::parse(modifier).unwrap(),
                            5.0,
                            AttributeOperation::AddValue
                        ),
                        ModifierLifetime::Permanent,
                    ));
                    assert_eq!(
                        living.attribute_value(host, attribute),
                        Some(23.0),
                        "refresh applies the now-worn item"
                    );
                    commit.then_some(()).ok_or(())
                });
                assert_eq!(result.is_ok(), commit);
                assert_eq!(
                    place.resource(game),
                    if commit { after } else { Some(before) }
                );
                assert_eq!(
                    living.attribute_value(game, attribute),
                    Some(if commit { 23.0 } else { 25.0 })
                );
                let entry = saved_mob_attribute(game, entity, attribute);
                let permanent = Tag::List(vec![Tag::compound([
                    ("id", Tag::String(modifier.into())),
                    ("amount", Tag::Double(5.0f64.to_bits())),
                    ("operation", Tag::String("add_value".into())),
                ])]);
                assert_eq!(entry.get("modifiers"), (!commit).then_some(&permanent));
            });
        }
    }
    /// A living entity's [`ENTITY_ITEMS`] is its equipment (#932): seven slots by
    /// `EquipmentSlot`'s ordinal, the armour ones taking only what their `equippable`
    /// names for that kind, the writes transactional — an abort puts the equipment back
    /// and stays quiet, a commit is `set_equipment`'s write.
    #[test]
    #[ignore = "daily"]
    fn living_equipment_moves_through_the_entity_items_handler() {
        with_resident_game(|game| {
            let id = zombie(game);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(id)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            assert_eq!(items.size(host), 7);
            let chestplate = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let boots = ItemResource::of(
                &ItemStack::new("minecraft:diamond_boots", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let sword = ItemResource::of(
                &ItemStack::new("minecraft:iron_sword", 1)
                    .try_into_api()
                    .unwrap(),
            );
            // The armour slots take what names them for a zombie, one of it; the
            // chestplate is not for the feet, and a sword is not armour at all.
            assert_eq!(items.capacity(host, 4, Some(&chestplate)), 1);
            assert_eq!(items.capacity(host, 2, Some(&chestplate)), 0);
            assert_eq!(items.capacity(host, 2, Some(&boots)), 1);
            assert_eq!(items.capacity(host, 4, Some(&sword)), 0);
            // A hand takes a stack of anything.
            assert_eq!(items.capacity(host, 0, Some(&sword)), 1);

            // An aborted transaction puts the equipment back.
            let _ = host.transaction(|host, tx| {
                assert_eq!(items.insert_at(host, tx, 4, &chestplate, 1), Ok(1));
                assert_eq!(
                    items.amount(host, 4),
                    1,
                    "the transaction sees its own write"
                );
                Err::<(), ()>(())
            });
            assert_eq!(items.amount(host, 4), 0, "the abort put it back");

            // A committed one wears it and hands it back out.
            let _ = host.transaction(|host, tx| items.insert_at(host, tx, 0, &sword, 1));
            let _ = host.transaction(|host, tx| items.insert_at(host, tx, 4, &chestplate, 1));
            assert_eq!(items.amount(host, 0), 1);
            assert_eq!(items.amount(host, 4), 1);
            let _ = host.transaction(|host, tx| items.extract_at(host, tx, 4, &chestplate, 1));
            assert_eq!(items.amount(host, 4), 0);
            assert_eq!(items.amount(host, 0), 1);

            // A gone entity answers nothing.
            game.mobs.clear();
            let host: &mut dyn ServerHost = game;
            assert_eq!(items.amount(host, 4), 0);
            assert_eq!(
                host.transaction(|host, tx| items.insert_at(host, tx, 0, &sword, 1)),
                Ok(0)
            );
        });
    }
    /// A committed equipment write through the handler is `set_equipment`'s: the same
    /// stacks, and the same attribute map (#941's live map reads the worn armour).
    #[test]
    #[ignore = "daily"]
    fn a_committed_equipment_write_matches_set_equipments() {
        use mod_api::EquipmentSlot;
        with_resident_game(|game| {
            zombie(game);
            game.common.mobs.push(sunrise_sim::mobs::Mob::new(
                31,
                game.common
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone(),
                sunrise_core::geometry::Vec3::new(2.5, 64.0, 0.5),
                0.0,
            ));
            let host: &mut dyn ServerHost = game;
            let chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            // Through the handler, committed.
            let handler = host
                .entity(30)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let _ = host.transaction(|host, tx| {
                handler.insert_at(
                    host,
                    tx,
                    4,
                    &ItemResource::of(&chest.clone().try_into_api().unwrap()),
                    1,
                )
            });
            // Through `set_equipment`.
            let through_setter = game.living(game.entity(31).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                through_setter,
                EquipmentSlot::Chest,
                Some(chest.clone())
            ));
            let (a, b) = (&game.mobs[0], &game.mobs[1]);
            assert_eq!(
                a.equipment_stacks().get(4).cloned(),
                b.equipment_stacks().get(4).cloned(),
                "the same worn stack"
            );
            // The attribute map answers the same armour for both (#941).
            let read = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                let host: &dyn mod_api::Host = game;
                living.attribute_value(host, "minecraft:armor")
            };
            assert_eq!(read(game, 30), read(game, 31));
            assert!(
                read(game, 30).unwrap_or(0.0) >= 8.0,
                "diamond chestplate armour"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_slot_extract_matches_resident_damage_without_max_damage() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_item_places::tests::player_slot_extract_matches_resident_damage_without_max_damage",
            || {
                with_game(|game| {
                    open_to_lan(game);
                    game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
                    let slot = ItemPlace::PlayerSlot {
                        player: 123,
                        slot: 3,
                    };
                    let mut paper = ItemStack::new("minecraft:paper", 2);
                    paper.damage = 40;
                    let mut explicit_default = paper.clone();
                    explicit_default.data_mut().set_other(
                        "minecraft:max_stack_size",
                        Some(sunrise_core::nbt::Tag::Int(64)),
                    );
                    let mut different_damage = explicit_default.clone();
                    different_damage.damage = 41;
                    let different_damage =
                        ItemResource::of(&different_damage.try_into_api().unwrap());
                    for (mut resident, equivalent) in [
                        (paper.clone(), paper.clone()),
                        (paper.clone(), explicit_default.clone()),
                        (explicit_default, paper),
                    ] {
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| {
                                guest.inventory[3] = Some(resident.clone());
                            });
                        let resource = slot.resource(game).unwrap();
                        assert_eq!(resource.as_stack().damage_component(), Some(40));
                        let removed = game
                            .transaction(|host, tx| slot.extract(host, tx, &different_damage, 1));
                        assert_eq!(removed, Ok(0));
                        assert_eq!(
                            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3],
                            Some(resident.clone()),
                        );

                        let equivalent = ItemResource::of(&equivalent.try_into_api().unwrap());
                        let removed =
                            game.transaction(|host, tx| slot.extract(host, tx, &equivalent, 1));
                        assert_eq!(removed, Ok(1));
                        resident.count = 1;
                        assert_eq!(
                            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3],
                            Some(resident),
                        );
                    }
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn player_slot_extracts_a_map_by_its_pending_lock_resource() {
        use mod_api::{ComponentId, Tag};

        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_item_places::tests::player_slot_extracts_a_map_by_its_pending_lock_resource",
            "SUNRISE_ITEM_RESOURCE_MAP_LOCK_CASE",
            &["run"],
        ) {
            return;
        }

        with_game(|game| {
            open_to_lan(game);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let mut map = ItemStack::new("minecraft:filled_map", 1);
            map.data_mut().map_id = Some(7);
            let no_directive = ItemResource::of(&map.try_into_api().unwrap());
            map.data_mut().map_post_processing = Some(0);
            let mut player = guest(1);
            player.inventory[3] = Some(map.clone());
            game.server.lan.as_mut().unwrap().hub.guests.push(player);

            assert_eq!(
                game.transaction(|host, tx| slot.extract(host, tx, &no_directive, 1)),
                Ok(0)
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3],
                Some(map)
            );

            let resource = slot.resource(game).unwrap();
            let post_processing = ComponentId::new("minecraft:map_post_processing").unwrap();
            assert_eq!(resource.component(&post_processing), Some(Tag::Int(0)));
            assert_eq!(
                game.transaction(|host, tx| slot.extract(host, tx, &resource, 1)),
                Ok(1)
            );
            assert!(game.server.lan.as_ref().unwrap().hub.guests[0].inventory[3].is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_creative_players_selected_stack_takes_what_fits_and_keeps_it() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.game_mode = 1;
            player.inventory[0] = Some(ItemStack::new("minecraft:stick", 60));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            assert_eq!(
                host.transaction(|host, tx| items.insert(host, tx, &sticks, 4)),
                Ok(4)
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                64
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_kept_handler_reaches_nothing_once_its_cart_is_removed() {
        use sunrise_gameplay_common::game::commands_common::Target;
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.carts.push(Minecart::new(
                500,
                CartKind::Chest,
                sunrise_core::geometry::Vec3::ZERO,
            ));
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(500).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            game.persist.removals.push(Target::Cart(500));
            assert!(entity.capability(game, &ENTITY_ITEMS, ()).is_none());
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let host: &mut dyn ServerHost = game;
            let wrote = host.transaction(|host, tx| items.insert_at(host, tx, 0, &sticks, 1));
            assert_eq!(wrote, Ok(0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn bound_armour_comes_off_only_in_creative() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            helmet
                .data_mut()
                .enchantments
                .push(("minecraft:binding_curse".into(), 1));
            player.armor[0] = Some(helmet.clone());
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let helmet = ItemResource::of(&helmet.try_into_api().unwrap());
            assert_eq!(items.amount(host, 39), 1);
            let take = |host: &mut dyn ServerHost| {
                host.transaction(|host, tx| items.extract_at(host, tx, 39, &helmet, 1))
            };
            assert_eq!(take(host), Ok(0));
            // A creative player takes it off (`ArmorSlot.mayPickup`'s `isCreative`).
            {
                let value = 1;
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            assert_eq!(take(game), Ok(1));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].armor[0],
                None
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn an_equippable_whose_entity_tag_holds_the_player_fits_its_slot() {
        use mod_api::{ComponentId, Tag};
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let helmet = ItemResource::of(
                &ItemStack::new("minecraft:iron_helmet", 1)
                    .try_into_api()
                    .unwrap(),
            )
            .with_component(
                host,
                &ComponentId::new("minecraft:equippable").unwrap(),
                Tag::Compound(
                    [
                        ("slot".into(), Tag::String("head".into())),
                        (
                            "allowed_entities".into(),
                            Tag::String("#minecraft:cannot_be_pushed_onto_boats".into()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            )
            .unwrap();
            assert_eq!(items.capacity(host, 39, Some(&helmet)), 1);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_kept_handler_reaches_nothing_once_its_world_closes() {
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.carts.push(Minecart::new(
                500,
                CartKind::Chest,
                sunrise_core::geometry::Vec3::ZERO,
            ));
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(500).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            game.native_lifecycle.begin_close();
            game.native_lifecycle.retire();
            assert!(entity.capability(game, &ENTITY_ITEMS, ()).is_none());
            let host: &mut dyn ServerHost = game;
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let wrote = host.transaction(|host, tx| items.insert_at(host, tx, 0, &sticks, 1));
            assert_eq!(wrote, Ok(0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn crafted_by_processes_a_held_map_and_defers_the_stat_and_new_map_until_commit() {
        use sunrise_gameplay_common::game::maps::MapData;

        super::super::native_guest_turn_tests::with_guests(|game| {
            let source = game.new_map_id(MapData::fresh(0.0, 0.0, 0, "minecraft:overworld"));
            let mut map = ItemStack::new("minecraft:filled_map", 1);
            map.data_mut().map_id = Some(source);
            map.data_mut().map_post_processing = Some(1);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.carried = Some(map.clone()));
            let menu = game
                .player_container_menu(game.player(12).unwrap())
                .unwrap();
            let cursor = menu.cursor();
            let other = ItemPlace::PlayerSlot {
                player: 12,
                slot: 0,
            };
            let host: &mut dyn ServerHost = game;
            let aborted = host.transaction(|host, tx| {
                assert_eq!(
                    other.on_crafted_by(host, tx, 1),
                    Err(CraftedByError::NotHeld)
                );
                cursor.on_crafted_by(host, tx, 3)?;
                Err::<(), CraftedByError>(CraftedByError::Unavailable)
            });
            assert_eq!(
                aborted,
                Err(TransactionError::Aborted(CraftedByError::Unavailable))
            );
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.carried, Some(map.clone()));
            let conn = guest.conn;
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .play
                    .get(&conn)
                    .unwrap()
                    .stats
                    .get("crafted", "minecraft:filled_map"),
                0
            );
            assert!(game.map(source + 1).is_none());

            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| cursor.on_crafted_by(host, tx, 3)),
                Ok(())
            );
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            let carried = guest.carried.as_ref().unwrap();
            assert_eq!(
                carried.data().and_then(|data| data.map_id),
                Some(source + 1)
            );
            assert_eq!(
                carried.data().and_then(|data| data.map_post_processing),
                None
            );
            assert_eq!(game.map(source + 1).unwrap().scale, 1);
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .play
                    .get(&conn)
                    .unwrap()
                    .stats
                    .get("crafted", "minecraft:filled_map"),
                3
            );

            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:stick", 2))
                });
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| cursor.on_crafted_by(host, tx, 2)),
                Ok(())
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:stick", 2))
            );
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .play
                    .get(&conn)
                    .unwrap()
                    .stats
                    .get("crafted", "minecraft:stick"),
                2
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn crafted_by_map_reservations_follow_nested_commit_and_abort() {
        use sunrise_core::nbt::Tag;
        use sunrise_sim::menu::Menu;
        use sunrise_sim::save::read_dat;
        use {
            sunrise_gameplay_common::game::maps::MapData,
            sunrise_gameplay_common::game::maps::map_id,
        };

        super::super::native_guest_turn_tests::with_guests(|game| {
            let save = tempfile::tempdir().unwrap();
            game.server.world_root = save.path().to_owned();
            let maps_dir = save.path().join("data/minecraft/maps");
            let source = game.new_map_id(MapData::fresh(0.0, 0.0, 0, "minecraft:overworld"));
            let map = |directive| {
                let mut stack = ItemStack::new("minecraft:filled_map", 1);
                stack.data_mut().map_id = Some(source);
                stack.data_mut().map_post_processing = Some(directive);
                stack
            };
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.carried = Some(map(1));
                    guest.inventory[0] = Some(map(0));
                });
            let menu = game
                .player_container_menu(game.player(12).unwrap())
                .unwrap();
            let cursor = menu.cursor();
            let slot = ItemPlace::MenuSlot { menu, slot: 36 };
            let held_ids = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                [
                    game.menu_carried(menu).flatten().as_ref().and_then(map_id),
                    game.menu_slot(menu, 36).flatten().as_ref().and_then(map_id),
                ]
            };
            let saved_counter = || {
                let index = read_dat(&maps_dir.join("last_id.dat")).unwrap();
                index.get("data").unwrap().get("map").unwrap().clone()
            };

            let _ = game.storage_transaction(|game, tx| {
                cursor.on_crafted_by(game, tx, 1).unwrap();
                let _ = game.storage_transaction(|game, child_tx| {
                    slot.on_crafted_by(game, child_tx, 1).unwrap();
                    assert_eq!(held_ids(game), [Some(source + 1), Some(source + 2)]);

                    // A held map derived from the parent's pending copy can find the new map.
                    let mut derived = game.menu_carried(menu).flatten().unwrap();
                    derived.data_mut().map_post_processing = Some(0);
                    let maps = game.level_maps([&derived]);
                    Menu::map_crafted(Some(&maps), &mut derived);
                    assert_eq!(map_id(&derived), Some(source + 3));
                    game.register_made_maps(maps);
                    let chained = game.map(source + 3).unwrap();
                    assert_eq!(chained.scale, 1);
                    assert!(chained.locked);
                    Err::<(), ()>(())
                });
                assert_eq!(held_ids(game), [Some(source + 1), Some(source)]);
                assert!(game.map(source + 2).is_none());
                assert!(game.map(source + 3).is_none());

                game.storage_transaction(|game, child_tx| {
                    slot.on_crafted_by(game, child_tx, 1).unwrap();
                    Ok::<(), ()>(())
                })
                .unwrap();
                assert_eq!(held_ids(game), [Some(source + 1), Some(source + 2)]);
                assert!(game.map(source + 2).unwrap().locked);

                // Child commit still isn't a save boundary. Root abort restores both slots and IDs.
                game.save_maps();
                assert_eq!(saved_counter(), Tag::Int(source));
                for id in [source + 1, source + 2, source + 3] {
                    assert!(!maps_dir.join(format!("{id}.dat")).exists());
                }
                Err::<(), ()>(())
            });
            game.save_maps();
            assert_eq!(saved_counter(), Tag::Int(source));
            assert_eq!(game.peek_map_id(), source + 1);
            assert_eq!(game.menu_carried(menu).flatten(), Some(map(1)));
            assert_eq!(game.menu_slot(menu, 36).flatten(), Some(map(0)));
            for id in [source + 1, source + 2, source + 3] {
                assert!(game.map(id).is_none());
                assert!(!maps_dir.join(format!("{id}.dat")).exists());
            }

            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.transaction(|host, tx| {
                    cursor.on_crafted_by(host, tx, 1)?;
                    slot.on_crafted_by(host, tx, 1)
                }),
                Ok(())
            );
            assert_eq!(held_ids(game), [Some(source + 1), Some(source + 2)]);
            assert_eq!(game.map(source + 1).unwrap().scale, 1);
            assert!(game.map(source + 2).unwrap().locked);
            game.save_maps();
            assert_eq!(saved_counter(), Tag::Int(source + 2));
            assert!(maps_dir.join(format!("{}.dat", source + 1)).is_file());
            assert!(maps_dir.join(format!("{}.dat", source + 2)).is_file());
            assert!(!maps_dir.join(format!("{}.dat", source + 3)).exists());
        });
    }
    pub(crate) fn with_crafted_by_handler_map(
        accepts_replacement: bool,
        on_insert: Option<sunrise_sim::lan::PlayerSender>,
        test: impl FnOnce(
            &mut sunrise_gameplay::game::ServerGame<'_>,
            ItemPlace,
            i32,
            mod_api::ItemStack,
        ),
    ) {
        use mod_api::{
            AttachedItems, AttachmentDescriptor, AttachmentKey, Holder, MenuReplace, MenuSlotPlace,
            ResourceHandler, StackList, TransactionContext,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_core::nbt::{JavaString, Tag};
        use sunrise_gameplay_common::game::maps::MapData;

        struct MapSlotHandler {
            inner: AttachedItems,
            on_insert: Option<sunrise_sim::lan::PlayerSender>,
        }

        impl ResourceHandler<ItemResource> for MapSlotHandler {
            fn size(&self, host: &mut dyn ServerHost) -> usize {
                self.inner.size(host)
            }
            fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
                self.inner.resource(host, index)
            }
            fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
                self.inner.amount(host, index)
            }
            fn capacity(
                &self,
                host: &mut dyn ServerHost,
                index: usize,
                resource: Option<&ItemResource>,
            ) -> u32 {
                self.inner.capacity(host, index, resource)
            }
            fn is_valid(
                &self,
                host: &mut dyn ServerHost,
                index: usize,
                resource: &ItemResource,
            ) -> bool {
                self.inner.is_valid(host, index, resource)
            }
            fn insert_at(
                &self,
                host: &mut dyn ServerHost,
                tx: &TransactionContext,
                index: usize,
                resource: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
                let inserted = self.inner.insert_at(host, tx, index, resource, amount)?;
                if inserted > 0
                    && let Some(sender) = &self.on_insert
                {
                    sender.send(vec![1]);
                }
                Ok(inserted)
            }
            fn extract_at(
                &self,
                host: &mut dyn ServerHost,
                tx: &TransactionContext,
                index: usize,
                resource: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
                self.inner.extract_at(host, tx, index, resource, amount)
            }
        }

        struct MapStorage {
            accepts_replacement: bool,
            slot: Arc<Mutex<Option<AttachmentKey<StackList<ItemResource>>>>>,
            on_insert: Option<sunrise_sim::lan::PlayerSender>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for MapStorage {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let slot = r
                    .register_attachment_type(AttachmentDescriptor::new(
                        "crafted_map_fixture:slot",
                        StackList::<ItemResource>::new,
                    ))
                    .unwrap();
                *self.slot.lock().unwrap() = Some(slot.clone());
                let valid: fn(&ItemResource) -> bool = if self.accepts_replacement {
                    |_| true
                } else {
                    |_| false
                };
                let on_insert = self.on_insert.clone();
                r.entity_capability_before_game(
                    &ENTITY_ITEMS,
                    "minecraft:pig",
                    move |_, entity, ()| {
                        Some(Box::new(MapSlotHandler {
                            inner: AttachedItems::single(
                                &slot,
                                Holder::Entity(entity.id()),
                                1,
                                valid,
                            ),
                            on_insert: on_insert.clone(),
                        }))
                    },
                );
            }
        }

        let slot = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "crafted_map_fixture",
            Box::new(MapStorage {
                accepts_replacement,
                slot: slot.clone(),
                on_insert,
            }),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                let source = game.new_map_id(MapData::fresh(0.0, 0.0, 1, "minecraft:overworld"));
                let mut map = ItemStack::new("minecraft:filled_map", 1);
                map.data_mut().map_id = Some(source);
                let mut map = map.try_into_api().unwrap();
                map.apply_components(
                    &[
                        (
                            JavaString::from("minecraft:map_post_processing"),
                            Tag::String("scale".into()),
                        ),
                        (
                            JavaString::from("minecraft:custom_data"),
                            Tag::compound([("marker", Tag::Int(17))]),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )
                .unwrap();

                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                pig(game, 700, 64.0);
                let entity = game.entity(700).unwrap();
                let map_slot = slot.lock().unwrap().clone().expect("MapStorage registered");
                map_slot
                    .set(
                        game,
                        Holder::Entity(entity.id()),
                        StackList::from([(0, (ItemResource::of(&map), 1))]),
                    )
                    .unwrap();
                let player = game.player(12).unwrap();
                let host: &mut dyn ServerHost = game;
                let menu = player
                    .open_chest_menu(
                        host,
                        3,
                        mod_api::Text::literal("map fixture"),
                        Vec::new(),
                        MenuReplace::CloseFirst,
                    )
                    .unwrap();
                menu.back_slots(host, vec![MenuSlotPlace::EntityItems { entity, slot: 0 }])
                    .unwrap();
                let place = ItemPlace::MenuSlot { menu, slot: 0 };
                assert_eq!(
                    place.with_stack(game, |stack| stack.project()).flatten(),
                    Some(map.clone())
                );
                test(game, place, source, map);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn crafted_by_replaces_the_map_in_a_handler_backed_slot() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_item_places::tests::crafted_by_replaces_the_map_in_a_handler_backed_slot",
            "SUNRISE_CRAFTED_BY_HANDLER_CASE",
            &["replace"],
        ) {
            return;
        }
        with_crafted_by_handler_map(true, None, |game, place, source, original| {
            assert_eq!(
                game.storage_transaction(|game, tx| place.on_crafted_by(game, tx, 3)),
                Ok(())
            );
            let updated = place
                .with_stack(game, |stack| stack.project())
                .flatten()
                .unwrap();
            use sunrise_core::nbt::JavaString;
            let patch = updated.components().unwrap().as_nbt();
            let custom_data = JavaString::from("minecraft:custom_data");
            assert_eq!(updated.item(), original.item());
            assert_eq!(updated.count(), original.count());
            assert_eq!(
                patch.get(&JavaString::from("minecraft:map_id")),
                Some(&mod_api::Tag::Int(source + 1))
            );
            assert!(!patch.contains_key(&JavaString::from("minecraft:map_post_processing")));
            assert_eq!(
                patch.get(&custom_data),
                original.components().unwrap().as_nbt().get(&custom_data)
            );
            assert_eq!(game.map(source + 1).unwrap().scale, 2);
            assert_eq!(game.map(source).unwrap().scale, 1);
            assert_eq!(
                game.server.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("crafted", "minecraft:filled_map"),
                3
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn crafted_by_refused_handler_write_drops_the_map_and_stat() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_item_places::tests::crafted_by_refused_handler_write_drops_the_map_and_stat",
            "SUNRISE_CRAFTED_BY_HANDLER_CASE",
            &["refuse"],
        ) {
            return;
        }
        with_crafted_by_handler_map(false, None, |game, place, source, original| {
            // Catch the refusal and commit the caller's transaction anyway.
            assert_eq!(
                game.storage_transaction(|game, tx| Ok::<_, ()>(place.on_crafted_by(game, tx, 3))),
                Ok(Err(CraftedByError::Unavailable))
            );
            assert_eq!(
                place.with_stack(game, |stack| stack.project()).flatten(),
                Some(original)
            );
            assert!(game.map(source + 1).is_none());
            assert_eq!(game.peek_map_id(), source + 1);
            let crafted = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .play
                .get(&12)
                .map_or(0, |play| play.stats.get("crafted", "minecraft:filled_map"));
            assert_eq!(crafted, 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn crafted_by_packet_refusal_returns_commit_refused_and_drops_map_and_stat() {
        use std::sync::{atomic::AtomicUsize, mpsc};
        use sunrise_gameplay_common::game::maps::MapData;
        use sunrise_sim::lan::{PlayerSender, conn::transaction_packets::Scope};

        const CASE: &str = "SUNRISE_CRAFTED_BY_PACKET_REFUSAL_CASE";
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_item_places::tests::crafted_by_packet_refusal_returns_commit_refused_and_drops_map_and_stat",
            CASE,
            &["exchange", "settle"],
        ) {
            return;
        }

        let (send, receive) = mpsc::channel();
        // A real bounded LAN sender with no room left to reserve another packet.
        let sender = PlayerSender::new(send, Arc::new(AtomicUsize::new(usize::MAX)));
        if std::env::var(CASE).unwrap() == "settle" {
            super::super::native_guest_turn_tests::with_guests(|game| {
                let source = game.new_map_id(MapData::fresh(0.0, 0.0, 1, "minecraft:overworld"));
                let mut original = ItemStack::new("minecraft:filled_map", 1);
                original.data_mut().map_id = Some(source);
                original.data_mut().map_post_processing = Some(1);
                game.as_guest(12, |_| {}).unwrap(); // Initialize the parked guest's body.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(original.clone());
                        guest.sender = sender;
                        guest.motion.as_mut().unwrap().health.indicated_dir = Some(90.0);
                    });
                let menu = game
                    .player_container_menu(game.player(12).unwrap())
                    .unwrap();
                let _packets = Scope::begin();
                let outer = game.storage_transaction_settled(|game, tx| {
                    // No ItemPlace::exchange: the cursor is written in the guest turn, then that
                    // turn sends its pending hurt animation on return. The write succeeds and
                    // packet refusal is first observed when the replacement frame settles.
                    assert_eq!(
                        menu.cursor().on_crafted_by(game, tx, 3),
                        Err(CraftedByError::CommitRefused)
                    );
                    assert!(Scope::refused());
                    assert_eq!(game.pending_transaction_item_stats(), 0);
                    assert_eq!(game.menu_carried(menu), Some(Some(original.clone())));
                    assert!(game.map(source + 1).is_none());
                    assert_eq!(game.peek_map_id(), source + 1);
                    Ok::<_, ()>(())
                });
                assert_eq!(outer, Err(TransactionError::CommitRefused));
                assert_eq!(game.menu_carried(menu), Some(Some(original)));
                assert!(game.map(source + 1).is_none());
                assert_eq!(game.peek_map_id(), source + 1);
                assert_eq!(
                    game.server.lan.as_ref().unwrap().play[&12]
                        .stats
                        .get("crafted", "minecraft:filled_map"),
                    0
                );
                assert!(receive.try_recv().is_err());
            });
            return;
        }

        with_crafted_by_handler_map(true, Some(sender), |game, place, source, original| {
            let _packets = Scope::begin();
            let outer = game.storage_transaction_settled(|game, tx| {
                let crafted = place.on_crafted_by(game, tx, 3);
                assert_eq!(crafted, Err(CraftedByError::CommitRefused));
                assert_eq!(game.pending_transaction_item_stats(), 0);
                assert_eq!(
                    place.with_stack(game, |stack| stack.project()).flatten(),
                    Some(original.clone())
                );
                assert!(game.map(source + 1).is_none());
                assert_eq!(game.peek_map_id(), source + 1);
                Ok::<_, ()>(())
            });
            assert_eq!(outer, Err(TransactionError::CommitRefused));
            assert_eq!(
                place.with_stack(game, |stack| stack.project()).flatten(),
                Some(original)
            );
            assert!(game.map(source + 1).is_none());
            assert_eq!(game.peek_map_id(), source + 1);
            let crafted = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .play
                .get(&12)
                .map_or(0, |play| play.stats.get("crafted", "minecraft:filled_map"));
            assert_eq!(crafted, 0);
            assert!(receive.try_recv().is_err());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_chest_menus_slots_write_the_chest_and_the_inventory_and_an_abort_puts_them_back() {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::lan::menu::Slots;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let host = game.server.lan.as_mut().unwrap();
            host.hub.guests.push(guest(1));
            let title = sunrise_core::nbt::Tag::String("chest".into());
            host.hub.open_container(
                1,
                "minecraft:generic_9x3",
                title,
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 27],
                None,
            );
            // Opening a window leaves the inventory's cursor parked; seed the chest menu's.
            host.hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:emerald", 9));
            });
            let menu = menu_of(game);
            // Slot 0 is the chest's first, 27 the inventory's 9 and 54 the hotbar's first.
            let (chest, inventory, hotbar) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 27 },
                ItemPlace::MenuSlot { menu, slot: 54 },
            );
            let held = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("no chest")
                };
                let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
                (
                    items[0].clone(),
                    g.inventory[9].clone(),
                    g.inventory[0].clone(),
                    g.carried.clone(),
                )
            };
            let before = held(game);
            let host: &mut dyn ServerHost = game;
            let emerald = ItemResource::of(
                &ItemStack::new("minecraft:emerald", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let moved = |host: &mut dyn ServerHost, commit: bool| {
                host.transaction(|host, tx| {
                    let took = menu.cursor().extract(host, tx, &emerald, 9).unwrap();
                    let went = [&chest, &inventory, &hotbar]
                        .map(|slot| slot.insert(host, tx, &emerald, 3).unwrap());
                    if commit {
                        Ok((took, went))
                    } else {
                        Err((took, went))
                    }
                })
            };
            assert_eq!(
                moved(host, false),
                Err(TransactionError::Aborted((9, [3, 3, 3])))
            );
            assert_eq!(held(game), before);
            let host: &mut dyn ServerHost = game;
            assert_eq!(moved(host, true), Ok((9, [3, 3, 3])));
            let three = Some(ItemStack::new("minecraft:emerald", 3));
            assert_eq!(held(game), (three.clone(), three.clone(), three, None));
            // A slot holds what its item stacks to and no more, and nothing spills elsewhere.
            assert_eq!(write(game, &chest, "minecraft:emerald", 70, true), 61);
            assert_eq!(write(game, &chest, "minecraft:stick", 1, true), 0);
            assert_eq!(write(game, &chest, "minecraft:emerald", -64, true), 64);
            assert_eq!(held(game).0, None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_chests_menu_over_the_furnace_set_in_its_place_reaches_none_of_its_slots() {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::lan::menu::Slots;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = |game: &sunrise_gameplay::game::ServerGame<'_>, name| {
                game.pack.assets.registry.default_state(name).unwrap()
            };
            let (chest, furnace) = (
                state(game, "minecraft:chest"),
                state(game, "minecraft:furnace"),
            );
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, flags),
                Ok(true)
            );
            let mut player = guest(1);
            player.inventory[8] = Some(ItemStack::new("minecraft:stick", 4));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let title = sunrise_core::nbt::Tag::String("chest".into());
            game.server.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x3",
                title,
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 27],
                None,
            );
            let menu = menu_of(game);
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], furnace, flags),
                Ok(true)
            );
            // Until the close pass, the menu still has the chest's 63 slots: 0 was the chest's
            // first, 62 is the last of the hotbar, past a furnace menu's 39.
            let (first, hotbar) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 62 },
            );
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            for place in [&first, &hotbar] {
                assert_eq!(write(game, place, "minecraft:stick", 1, true), 0);
                assert_eq!(write(game, place, "minecraft:stick", -1, true), 0);
                let host: &mut dyn ServerHost = game;
                let swapped = host.transaction(|host, tx| place.exchange(host, tx, &stick, 1));
                assert_eq!(swapped, Ok(0));
            }
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.inventory[8], Some(ItemStack::new("minecraft:stick", 4)));
            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&(0, 64, 0)) else {
                panic!("no furnace")
            };
            assert!(f.items.iter().all(Option::is_none));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn an_armour_slot_and_the_craft_result_take_only_what_the_inventory_menu_lets_them() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.craft_result = Some(ItemStack::new("minecraft:oak_planks", 4));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let menu = menu_of(game);
            // `InventoryMenu`: 0 the result, 5 the head, 8 the feet.
            let (result, head, feet) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 5 },
                ItemPlace::MenuSlot { menu, slot: 8 },
            );
            assert_eq!(write(game, &head, "minecraft:stick", 1, true), 0);
            assert_eq!(write(game, &feet, "minecraft:iron_helmet", 1, true), 0);
            assert_eq!(write(game, &head, "minecraft:iron_helmet", 2, true), 1);
            assert_eq!(write(game, &result, "minecraft:oak_planks", 1, true), 0);
            assert_eq!(write(game, &result, "minecraft:oak_planks", -4, true), 0);
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.armor[0], Some(ItemStack::new("minecraft:iron_helmet", 1)));
            assert_eq!(
                g.craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_crafting_grid_write_makes_its_result_at_the_commit() {
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let menu = menu_of(game);
            let grid = ItemPlace::MenuSlot { menu, slot: 1 };
            let log = ItemResource::of(
                &ItemStack::new("minecraft:oak_log", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let result = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .craft_result
                    .clone()
            };
            let host: &mut dyn ServerHost = game;
            let aborted = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), Ok(1));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!((g.craft_grid[0].clone(), result(game)), (None, None));
            let host: &mut dyn ServerHost = game;
            let committed = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), Ok(1));
                // Not yet: the result is made once, at the commit.
                Ok::<_, ()>(menu.slot(host, 0))
            });
            assert_eq!(committed, Ok(Some(None)));
            assert_eq!(
                result(game),
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
            assert_eq!(write(game, &grid, "minecraft:oak_log", -1, true), 1);
            assert_eq!(result(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_mods_chest_menu_is_written_by_slot_and_a_replaced_menu_writes_nothing() {
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let title = sunrise_core::nbt::Tag::String("shop".into());
            let open = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.open_native_menu(1, 1, title.clone(), vec![]).unwrap();
                menu_of(game)
            };
            let shop = ItemPlace::MenuSlot {
                menu: open(game),
                slot: 2,
            };
            let shown = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0].native_menu[2].clone()
            };
            assert_eq!(write(game, &shop, "minecraft:diamond", 5, false), 5);
            assert_eq!(shown(game), None);
            assert_eq!(write(game, &shop, "minecraft:diamond", 5, true), 5);
            assert_eq!(shown(game), Some(ItemStack::new("minecraft:diamond", 5)));
            // The same chest opened again is a new menu: the old one reaches nothing.
            let again = open(game);
            assert_eq!(write(game, &shop, "minecraft:diamond", 1, true), 0);
            assert_eq!(write(game, &shop, "minecraft:diamond", -1, true), 0);
            let host: &mut dyn ServerHost = game;
            assert_eq!(shop.with_stack(host, |_| ()), None);
            let fresh = ItemPlace::MenuSlot {
                menu: again,
                slot: 2,
            };
            assert_eq!(write(game, &fresh, "minecraft:diamond", 1, true), 1);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_furnace_menus_fuel_slot_takes_one_bucket_as_its_slot_does() {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::lan::menu::Slots;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let furnace = game.pack.assets.registry.default_state("minecraft:furnace");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], furnace.unwrap(), flags),
                Ok(true)
            );
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            game.server.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:furnace",
                sunrise_core::nbt::Tag::String("furnace".into()),
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 3],
                None,
            );
            let menu = menu_of(game);
            let (fuel, result) = (
                ItemPlace::MenuSlot { menu, slot: 1 },
                ItemPlace::MenuSlot { menu, slot: 2 },
            );
            // `FurnaceFuelSlot.getMaxStackSize(bucket)` is 1, not a bucket's 16; coal stacks to 64.
            assert_eq!(write(game, &fuel, "minecraft:bucket", 16, true), 1);
            assert_eq!(write(game, &fuel, "minecraft:bucket", 1, true), 0);
            assert_eq!(write(game, &fuel, "minecraft:bucket", -1, true), 1);
            assert_eq!(write(game, &fuel, "minecraft:stick", 70, true), 64);
            // `FurnaceResultSlot.mayPlace` is false.
            assert_eq!(write(game, &result, "minecraft:iron_ingot", 1, true), 0);
            let host: &mut dyn ServerHost = game;
            let bucket = ItemStack::new("minecraft:bucket", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(menu.slot_holds(host, 1, &bucket), Some(1));
            assert_eq!(menu.slot_holds(host, 2, &bucket), Some(0));
            // The hotbar's last slot is plain; past it there is no slot.
            assert_eq!(menu.slot_holds(host, 38, &bucket), Some(16));
            assert_eq!(menu.slot_holds(host, 39, &bucket), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stale_menus_cursor_reaches_nothing_of_the_next_ones() {
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let stale = menu_of(game);
            let title = sunrise_core::nbt::Tag::String("new".into());
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.open_native_menu(1, 1, title, vec![]).unwrap();
            {
                let value = Some(ItemStack::new("minecraft:stick", 1));
                hub.guests.update(0, |guest| guest.carried = value);
            }
            assert_eq!(stale.carried(game), None);
            assert_eq!(write(game, &stale.cursor(), "minecraft:stick", -1, true), 0);
            assert_eq!(write(game, &stale.cursor(), "minecraft:stick", 1, true), 0);
            let host: &mut dyn ServerHost = game;
            assert_eq!(stale.cursor().with_stack(host, |_| ()), None);
            let fresh = menu_of(game).cursor();
            assert_eq!(write(game, &fresh, "minecraft:stick", -1, true), 1);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_grid_written_in_and_around_an_aborted_child_makes_its_result_from_what_stays() {
        with_game(|game| {
            open_to_lan(game);
            game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let menu = menu_of(game);
            let (grid, other) = (
                ItemPlace::MenuSlot { menu, slot: 1 },
                ItemPlace::MenuSlot { menu, slot: 2 },
            );
            let log = ItemResource::of(
                &ItemStack::new("minecraft:oak_log", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            let committed = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), Ok(1));
                let child = host.transaction(|host, tx| {
                    assert_eq!(other.insert(host, tx, &log, 3), Ok(3));
                    Err::<(), ()>(())
                });
                assert!(child.is_err());
                assert_eq!((grid.amount(host), other.amount(host)), (1, 0));
                assert_eq!(grid.insert(host, tx, &log, 1), Ok(1));
                Ok::<_, ()>(menu.slot(host, 0))
            });
            assert_eq!(committed, Ok(Some(None)));
            // Two logs in one slot: one craft's planks; the child's second log never landed.
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.craft_grid[1], None);
            assert_eq!(
                g.craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_players_items_go_where_inventory_add_puts_them_and_an_abort_puts_them_back() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.selected = 4;
            player.inventory[4] = Some(ItemStack::new("minecraft:stick", 60));
            player.inventory[7] = Some(ItemStack::new("minecraft:stick", 10));
            player.inventory[2] = Some(ItemStack::new("minecraft:stone", 1));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let before = inventories(game);
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(20).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            assert_eq!(items.size(host), 41);
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let aborted = host.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, &sticks, 70), Ok(70));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(inventories(game), before);

            // `Inventory.add`: onto the selected stack (4 more), then the same item in order
            // (54 more), then the first empty slot (the other 12); not onto the stone.
            let host: &mut dyn ServerHost = game;
            let went = host.transaction(|host, tx| items.insert(host, tx, &sticks, 70));
            assert_eq!(went, Ok(70));
            let after = &inventories(game)[0];
            assert_eq!(after[4], Some(ItemStack::new("minecraft:stick", 64)));
            assert_eq!(after[7], Some(ItemStack::new("minecraft:stick", 64)));
            assert_eq!(after[0], Some(ItemStack::new("minecraft:stick", 12)));
            assert_eq!(after[2], Some(ItemStack::new("minecraft:stone", 1)));
            // Index 39 is the head: a stick doesn't go there.
            let host: &mut dyn ServerHost = game;
            let head = host.transaction(|host, tx| items.insert_at(host, tx, 39, &sticks, 1));
            assert_eq!(head, Ok(0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_trade_between_two_players_moves_both_offers() {
        with_game(|game| {
            traders(game, None);
            let (diamonds, emeralds) = offers();
            assert_eq!(trade(game, &diamonds, &emeralds), Ok(()));
            // The diamonds go first, into the buyer's first empty slot (1: the emeralds are
            // still in 0), then the emeralds into the seller's emptied slot 0.
            let after = inventories(game);
            assert_eq!(after[0][0], Some(ItemStack::new("minecraft:emerald", 5)));
            assert_eq!(after[1][0], None);
            assert_eq!(after[1][1], Some(ItemStack::new("minecraft:diamond", 3)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_trade_into_a_full_inventory_moves_nothing() {
        with_game(|game| {
            // The buyer's other slots and off hand hold stone, so only slot 0 is free once the
            // emeralds leave; the diamonds go before the emeralds and find no room.
            traders(game, Some(ItemStack::new("minecraft:stone", 64)));
            let before = inventories(game);
            let (diamonds, emeralds) = offers();
            assert_eq!(trade(game, &diamonds, &emeralds), Err(TradeRefused));
            assert_eq!(inventories(game), before);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_stale_offer_moves_nothing() {
        with_game(|game| {
            traders(game, None);
            let (diamonds, emeralds) = offers();
            // The seller used a diamond after offering three, and the buyer moved the emeralds
            // to another slot: each offer no longer matches its index.
            let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 2))
            });
            let before = inventories(game);
            assert_eq!(trade(game, &diamonds, &emeralds), Err(TradeRefused));
            assert_eq!(inventories(game), before);
            let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3))
            });
            guests.update(1, |g| g.inventory[4] = g.inventory[0].take());
            let before = inventories(game);
            assert_eq!(trade(game, &diamonds, &emeralds), Err(TradeRefused));
            assert_eq!(inventories(game), before);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_boat_slots_use_the_open_menu_and_refuse_a_removed_owner() {
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::boats_common::Boat;
        with_game(|game| {
            let mut boat = Boat::new(600, "oak".into(), Vec3::new(1.0, 65.0, 2.0), 0.0);
            boat.chest = true;
            boat.items.resize(27, None);
            game.boats.push(boat);
            assert_eq!(
                game.edit_entity_slots(600, |slots, at| {
                    assert_eq!(at, [1.0, 65.0, 2.0]);
                    slots[0] = Some(ItemStack::new("minecraft:stone", 4));
                    slots.len()
                }),
                Some(27)
            );
            let key = (
                i32::MAX - 15,
                sunrise_gameplay_common::game::minecarts::CART_KEY_Y,
                600,
            );
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
            game.edit_entity_slots(600, |slots, _| {
                assert!(slots[0].is_none());
                slots[0] = Some(ItemStack::new("minecraft:sand", 7));
            });
            assert_eq!(
                game.boats[0].items[0],
                Some(ItemStack::new("minecraft:stone", 4))
            );
            assert_eq!(
                game.edit_entity_slots(600, |slots, _| slots[0].clone()),
                Some(Some(ItemStack::new("minecraft:sand", 7)))
            );
            game.boats.remove(0);
            assert_eq!(
                game.edit_entity_slots(600, |_, _| panic!("stale owner reached its menu")),
                None::<()>
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_chest_minecarts_items_are_its_slots() {
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let position = sunrise_core::geometry::Vec3::new(0.5, 65.0, 0.5);
            game.carts
                .push(Minecart::new(500, CartKind::Chest, position));
            let host: &mut dyn ServerHost = game;
            let cart = host.entity(500).unwrap();
            assert_eq!(
                cart.kind(host).and_then(|kind| kind.id(host)).as_deref(),
                Some("minecraft:chest_minecart")
            );
            let items = cart.capability(host, &ENTITY_ITEMS, ()).unwrap();
            assert_eq!(items.size(host), 27);
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let aborted = host.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, &sticks, 70), Ok(70));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert!(game.carts[0].items.iter().all(Option::is_none));
            let host: &mut dyn ServerHost = game;
            let went = host.transaction(|host, tx| items.insert(host, tx, &sticks, 70));
            assert_eq!(went, Ok(70));
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stick", 64))
            );
            assert_eq!(
                game.carts[0].items[1],
                Some(ItemStack::new("minecraft:stick", 6))
            );
            // A plain minecart has no items.
            game.carts
                .push(Minecart::new(501, CartKind::Rideable, position));
            let host: &mut dyn ServerHost = game;
            let plain = host.entity(501).unwrap();
            assert!(plain.capability(host, &ENTITY_ITEMS, ()).is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn reading_and_writing_vehicle_slots_unpack_pending_loot() {
        use sunrise_core::{geometry::Vec3, nbt::Tag};
        use sunrise_gameplay_common::game::boats_common::Boat;
        use sunrise_gameplay_common::game::minecarts::{CART_KEY_Y, CartKind, Minecart};
        use sunrise_sim::save::PendingContainerLoot;
        with_game(|game| {
            game.load_server_loot();
            let pos = Vec3::new(1.0, 65.0, 2.0);
            let pending = PendingContainerLoot {
                table: Tag::String("minecraft:chests/simple_dungeon".into()),
                seed: Some(Tag::Long(42)),
            };
            let mut cart = Minecart::new(600, CartKind::Chest, pos);
            cart.loot = Some(pending);
            game.carts.push(cart);

            let _ = ItemPlace::EntitySlot {
                entity: 600,
                slot: 0,
            }
            .with_stack(game, |stack| stack.count());
            assert!(game.carts[0].loot.is_none());
            let (cart_slot, cart_rolled) = game.carts[0]
                .items
                .iter()
                .enumerate()
                .find_map(|(slot, item)| item.clone().map(|stack| (slot, stack)))
                .expect("reading a chest minecart slot unpacks its pending loot table");
            assert_eq!(
                ItemPlace::EntitySlot {
                    entity: 600,
                    slot: cart_slot,
                }
                .with_stack(game, |stack| stack.project())
                .flatten(),
                Some(cart_rolled.try_into_api().unwrap())
            );

            // An open vehicle menu still owns the live slots over the resident copy.
            let sand = ItemStack::new("minecraft:sand", 7);
            let mut open = vec![None; 27];
            open[2] = Some(sand.clone());
            game.world
                .block_entities
                .insert((i32::MAX - 15, CART_KEY_Y, 600), BlockEntity::Chest(open));
            assert_eq!(
                ItemPlace::EntitySlot {
                    entity: 600,
                    slot: 2,
                }
                .with_stack(game, |stack| stack.project())
                .flatten(),
                Some(sand.try_into_api().unwrap())
            );

            // Reading a chest boat slot unpacks its pending loot table and clears persist.raw tags.
            let mut read_boat = Boat::new(601, Arc::from("oak"), pos, 0.0);
            read_boat.chest = true;
            read_boat.items = vec![None; 27];
            game.boats.push(read_boat);
            game.persist.raw.insert(
                601,
                Tag::compound([
                    (
                        "LootTable",
                        Tag::String("minecraft:chests/simple_dungeon".into()),
                    ),
                    ("LootTableSeed", Tag::Long(42)),
                ]),
            );
            let _ = ItemPlace::EntitySlot {
                entity: 601,
                slot: 0,
            }
            .with_stack(game, |stack| stack.count());
            let read_raw = game.persist.raw.get(&601);
            assert!(read_raw.and_then(|tag| tag.get("LootTable")).is_none());
            assert!(read_raw.and_then(|tag| tag.get("LootTableSeed")).is_none());
            let (boat_slot, boat_rolled) = game.boats[0]
                .items
                .iter()
                .enumerate()
                .find_map(|(slot, item)| item.clone().map(|stack| (slot, stack)))
                .expect("reading a chest boat slot unpacks its pending loot table");
            assert_eq!(
                ItemPlace::EntitySlot {
                    entity: 601,
                    slot: boat_slot,
                }
                .with_stack(game, |stack| stack.project())
                .flatten(),
                Some(boat_rolled.try_into_api().unwrap())
            );

            // Writing via edit_entity_slots unpacks a chest boat's pending loot before running the edit.
            let mut write_boat = Boat::new(602, Arc::from("oak"), pos, 0.0);
            write_boat.chest = true;
            write_boat.items = vec![None; 27];
            game.boats.push(write_boat);
            game.persist.raw.insert(
                602,
                Tag::compound([
                    (
                        "LootTable",
                        Tag::String("minecraft:chests/simple_dungeon".into()),
                    ),
                    ("LootTableSeed", Tag::Long(42)),
                ]),
            );
            let saw_loot_before_edit = game
                .edit_entity_slots(602, |slots, _| slots.iter().any(Option::is_some))
                .expect("chest boat slots exist");
            assert!(saw_loot_before_edit);
            let write_raw = game.persist.raw.get(&602);
            assert!(write_raw.and_then(|tag| tag.get("LootTable")).is_none());
            assert!(write_raw.and_then(|tag| tag.get("LootTableSeed")).is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn player_cursor_read_ignores_guest_liveness() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            let paper = ItemStack::new("minecraft:paper", 3);
            player.dead = true;
            player.health = 0.0;
            player.carried = Some(paper.clone());
            game.server.lan.as_mut().unwrap().hub.guests.push(player);

            let menu = menu_of(game);
            let cursor = ItemPlace::PlayerCursor { menu };
            // Parked dead guest: cursor read still returns the carried stack.
            assert_eq!(
                cursor.with_stack(game, |stack| stack.project()).flatten(),
                Some(paper.clone().try_into_api().unwrap())
            );

            // Acting dead guest: cursor read still returns the carried stack from Game::carried.
            let acting_read = game
                .as_guest(1, |game| {
                    assert!(game.native_live_acting_body().is_none());
                    cursor.with_stack(game, |stack| stack.project()).flatten()
                })
                .expect("guest turn ran");
            assert_eq!(acting_read, Some(paper.try_into_api().unwrap()));

            // Empty cursor and air stack both read as None.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.carried = None);
            assert_eq!(cursor.with_stack(game, |_| ()), None);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:air", 1))
                });
            assert_eq!(cursor.with_stack(game, |_| ()), None);

            // Closed/stale menu still reads as None even when the guest carries a stack.
            let title = sunrise_core::nbt::Tag::String("new".into());
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.open_native_menu(1, 1, title, vec![]).unwrap();
            hub.guests.update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stick", 2))
            });
            assert_eq!(cursor.with_stack(game, |_| ()), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn cart_slot_edits_prefer_the_menu_and_refuse_a_removed_owner() {
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::minecarts::{CART_KEY_Y, CartKind, Minecart};
        with_game(|game| {
            let mut cart = Minecart::new(600, CartKind::Chest, Vec3::new(1.0, 65.0, 2.0));
            cart.items[0] = Some(ItemStack::new("minecraft:stone", 4));
            game.carts.push(cart);
            let key = (i32::MAX - 15, CART_KEY_Y, 600);
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
            assert_eq!(
                game.edit_cart_slots(600, |slots| {
                    slots[0] = Some(ItemStack::new("minecraft:sand", 7));
                    slots.len()
                }),
                Some(27)
            );
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stone", 4))
            );
            let Some(BlockEntity::Chest(open)) = game.world.block_entities.get(&key) else {
                panic!("missing live menu");
            };
            assert_eq!(open[0], Some(ItemStack::new("minecraft:sand", 7)));
            game.store_cart_items(key);
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:sand", 7))
            );
            game.edit_cart_slots(600, |slots| {
                slots[0] = Some(ItemStack::new("minecraft:gravel", 9))
            });
            game.close_cart_container(key);
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:gravel", 9))
            );
            assert!(!game.world.block_entities.contains_key(&key));
            game.carts.remove(0);
            assert_eq!(
                game.edit_cart_slots(600, |_| panic!("stale owner reached its menu")),
                None::<()>
            );
        });
    }
    /// Runs `test` with a mod answering `ENTITY_AUTOMATION` for pigs with `bin`.
    pub(crate) fn with_bin_pigs(
        bin: &Bin,
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        use mod_api::{self, Hooks, ModEntry, Mods, Registrar};
        struct Pigs(Bin);
        #[mod_api::__subscriptions]
        impl Hooks for Pigs {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let bin = self.0.clone();
                r.entity_capability(&ENTITY_AUTOMATION, "minecraft:pig", move |_, _, side| {
                    bin.sides.lock().unwrap().push(side);
                    Some(Box::new(bin.clone()))
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("pigs", Box::new(Pigs(bin.clone())))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || with_game(test));
    }
    #[test]
    #[ignore = "daily"]
    fn a_hopper_pushes_into_a_mod_entity_below_it_through_the_side_it_faces() {
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
            pig(game, 700, 63.0);
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 1))
            );
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 2))
            );
            // The hopper reaches the pig from above: its top.
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_hopper_pushes_into_a_block_with_no_block_entity_whose_provider_voids_the_items() {
        use mod_api::{self, Hooks, ModEntry, Mods, Registrar};
        /// A trash can's one slot: it takes anything and keeps nothing.
        struct Voids;
        impl mod_api::ResourceHandler<ItemResource> for Voids {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                None
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                0
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
                64
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &mod_api::TransactionContext,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
                Ok(amount)
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &mod_api::TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<mod_api::Overanswered>> {
                Ok(0)
            }
        }
        struct TrashCan;
        #[mod_api::__subscriptions]
        impl Hooks for TrashCan {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let lodestone = r.block_key("minecraft:lodestone");
                r.block_capability(&mod_api::ITEMS, &lodestone, |_, _, _, entity, offer| {
                    if entity.is_none() {
                        offer.offer(|| Box::new(Voids));
                    }
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("trash_can", Box::new(TrashCan))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
                let registry = &game.pack.assets.registry;
                let lodestone = registry.default_state("minecraft:lodestone").unwrap();
                let stone = registry.default_state("minecraft:stone").unwrap();
                game.world.set(0, 63, 0, lodestone).unwrap();
                game.world.set(1, 63, 0, stone).unwrap();
                // The hopper pushes one stick down into the lodestone, which keeps none.
                game.hopper_try_move_items((0, 64, 0));
                assert_eq!(
                    hopper_slot(game),
                    Some(ItemStack::new("minecraft:stick", 2))
                );
                let host: &mut dyn ServerHost = game;
                let level = host.level("minecraft:overworld").unwrap();
                let bin = level
                    .capability(
                        host,
                        &mod_api::ITEMS,
                        [0, 63, 0],
                        Some(mod_api::Direction::Up),
                    )
                    .expect("the lodestone's provider");
                assert_eq!(bin.owner(), "trash_can");
                assert_eq!(bin.amount(host, 0), 0);
                // Stone has no provider and no slots of the game's.
                assert!(
                    level
                        .capability(host, &mod_api::ITEMS, [1, 63, 0], None)
                        .is_none()
                );
            })
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_hopper_pulls_from_a_mod_entity_above_it_through_its_bottom() {
        let bin = Bin::default();
        *bin.held.lock().unwrap() = Some(("minecraft:stick".into(), 5));
        with_bin_pigs(&bin, |game| {
            hopper_with(game, "down", None);
            pig(game, 700, 65.0);
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 1))
            );
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 4))
            );
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Down)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_hopper_still_fills_a_chest_minecart_and_a_player_is_not_reached() {
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            open_to_lan(game);
            hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
            let position = sunrise_core::geometry::Vec3::new(0.5, 63.0, 0.5);
            game.carts
                .push(Minecart::new(500, CartKind::Chest, position));
            // A guest standing in the same cell, and no pig anywhere.
            let mut player = guest(1);
            player.position = [0.5, 63.0, 0.5];
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let player = host.entity(20).unwrap();
            assert!(
                player
                    .capability(host, &ENTITY_AUTOMATION, Some(Direction::Up))
                    .is_none()
            );
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stick", 1))
            );
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 2))
            );
            assert!(bin.held.lock().unwrap().is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_dropper_puts_into_a_mod_entity_in_front_of_it() {
        use sunrise_level::chunk::Chunk;
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dropper")
                .unwrap();
            let dropper = game.with_props(base, &[("facing", "down")]).unwrap();
            game.world.set(0, 64, 0, dropper).unwrap();
            let mut slots = vec![None; 9];
            slots[4] = Some(ItemStack::new("minecraft:stick", 2));
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(slots));
            pig(game, 700, 63.0);
            let dropped = game.dropped.len();
            game.dispense_from(sunrise_core::position::BlockPos::new(0, 64, 0), dropper)
                .unwrap();
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 1))
            );
            assert_eq!(game.dropped.len(), dropped);
            match game.world.block_entities.get(&(0, 64, 0)) {
                Some(BlockEntity::Chest(slots)) => {
                    assert_eq!(slots[4], Some(ItemStack::new("minecraft:stick", 1)));
                }
                _ => panic!("the dropper is gone"),
            }
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_crafter_puts_into_a_mod_entity_and_dispenses_only_what_it_refuses() {
        use mod_api::BlockUpdateFlags;
        use sunrise_core::{position::BlockPos, random::RandomSource as _};
        use sunrise_level::chunk::Chunk;

        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafter")
                .unwrap();
            let crafter = game
                .with_props(base, &[("orientation", "down_east")])
                .unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], crafter, BlockUpdateFlags::NONE),
                Ok(true)
            );
            pig(game, 700, 63.0);
            for (before, after, refused) in [(0, 4, 0), (62, 64, 2), (64, 64, 4)] {
                *bin.held.lock().unwrap() =
                    (before > 0).then(|| ("minecraft:stick".into(), before));
                let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get_mut(&(0, 64, 0))
                else {
                    panic!("the placed crafter is missing");
                };
                c.items[0] = Some(ItemStack::new("minecraft:oak_planks", 1));
                c.items[3] = Some(ItemStack::new("minecraft:oak_planks", 1));
                let dropped = game.dropped.len();
                let mut random = game.world.random.clone();
                game.dispense_from(BlockPos::new(0, 64, 0), crafter)
                    .unwrap();
                assert_eq!(
                    *bin.held.lock().unwrap(),
                    Some(("minecraft:stick".into(), after))
                );
                let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("the crafter is gone");
                };
                assert!(c.items.iter().all(Option::is_none));
                let output = &game.dropped[dropped..];
                if refused == 0 {
                    assert!(output.is_empty());
                    // One candidate-selection draw, including a single candidate, and no
                    // dispensing draws when the handler took the whole result.
                    random.next_int_bound(1).unwrap();
                    assert_eq!(game.world.random.clone().next_long(), random.next_long());
                } else {
                    assert_eq!(output.len(), 1);
                    assert_eq!(output[0].stack, ItemStack::new("minecraft:stick", refused));
                }
            }
            assert!(!game.world.block_entities.contains_key(&(0, 63, 0)));
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_mod_without_an_automation_provider_leaves_hoppers_on_the_vanilla_path() {
        use mod_api::{
            self, ENTITY_ENERGY, Hooks, ModEntry, Mods, Registrar, has_entity_providers,
        };
        use sunrise_core::random::RandomSource as _;
        struct Energy;
        #[mod_api::__subscriptions]
        impl Hooks for Energy {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.entity_capability(&ENTITY_ENERGY, "minecraft:pig", |_, _, _| None);
            }
        }
        let mods: Mods = vec![ModEntry::new("energy", Box::new(Energy))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            assert!(!has_entity_providers(&ENTITY_AUTOMATION));
            with_game(|game| {
                hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
                pig(game, 700, 63.0);
                let mut before = game.world.random.clone();
                game.hopper_try_move_items((0, 64, 0));
                // Nothing to reach, and no draw made: no container entity was a candidate.
                assert_eq!(
                    hopper_slot(game),
                    Some(ItemStack::new("minecraft:stick", 3))
                );
                assert_eq!(
                    game.world.random.clone().next_int_bound(1 << 30),
                    before.next_int_bound(1 << 30)
                );
            });
        });
    }
    /// #932, review: an armor stand's equipment through the same handler —
    /// a hopper dressing a stand — journaled like a mob's, an abort putting
    /// #932, review: an armor stand's equipment through the same handler —
    /// a hopper dressing a stand — journaled like a mob's, an abort putting
    /// it back and a commit matching `set_equipment`.
    #[test]
    #[ignore = "daily"]
    fn an_armor_stands_equipment_moves_through_the_handler() {
        use mod_api::EquipmentSlot;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ArmorStand;
        with_resident_game(|game| {
            game.armor_stands.push(ArmorStand::new(40, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(41, Vec3::ZERO, 0.0));
            let stand_slot = |game: &sunrise_gameplay::game::ServerGame<'_>, id: usize| {
                game.armor_stands[id].equipment.get(4).cloned()
            };
            let chestplate = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let helmet = ItemResource::of(
                &ItemStack::new("minecraft:iron_helmet", 1)
                    .try_into_api()
                    .unwrap(),
            );

            // Seven slots; the stand wears what its `equippable` names for
            // it, one of it; a sword is not armour.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                assert_eq!(items.size(host), 7);
                assert_eq!(items.capacity(host, 4, Some(&chestplate)), 1);
                assert_eq!(items.capacity(host, 5, Some(&helmet)), 1);
                assert_eq!(
                    items.capacity(
                        host,
                        4,
                        Some(&ItemResource::of(
                            &ItemStack::new("minecraft:iron_sword", 1)
                                .try_into_api()
                                .unwrap()
                        ))
                    ),
                    0
                );

                // An aborted insert leaves the stand as it was.
                let _ = host.transaction(|host, tx| {
                    assert_eq!(items.insert_at(host, tx, 4, &chestplate, 1), Ok(1));
                    assert_eq!(items.amount(host, 4), 1);
                    Err::<(), ()>(())
                });
                assert_eq!(items.amount(host, 4), 0, "the abort put it back");
            }
            assert_eq!(
                stand_slot(game, 0),
                None,
                "the stand's own slot is empty again"
            );

            // A committed one matches `set_equipment`.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                let _ = host.transaction(|host, tx| items.insert_at(host, tx, 4, &chestplate, 1));
            }
            let through_setter = game.living(game.entity(41).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                through_setter,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:diamond_chestplate", 1)),
            ));
            assert_eq!(
                stand_slot(game, 0),
                stand_slot(game, 1),
                "the same worn stack as set_equipment's"
            );

            // And it hands it back out.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                let came =
                    host.transaction(|host, tx| items.extract_at(host, tx, 4, &chestplate, 1));
                assert_eq!(came, Ok(1));
                assert_eq!(items.amount(host, 4), 0);
            }
            assert_eq!(stand_slot(game, 0), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_scratch_stack_lives_in_its_transaction_and_keeps_its_item() {
        use mod_api::{
            FLUIDS, ITEM_FLUIDS, ScratchError, StackWriteError, resource_util::move_resources,
        };
        with_game(|game| {
            let level = with_cauldron(game);
            let cauldron = game.world.block(2, 64, 0);
            let api = |name, count| ItemStack::new(name, count).try_into_api().unwrap();
            let bucket = api("minecraft:bucket", 1);
            let buckets = ItemResource::of(&bucket);
            let stick = ItemResource::of(&api("minecraft:stick", 1));
            let host: &mut dyn ServerHost = game;

            // An aborted loan discards its slot and leaves an escaped handle dead.
            let aborted = host.scratch_stack(bucket.clone(), |host, tx, place| {
                assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                assert_eq!(place.amount(host), 3);
                Err::<(), _>(place.clone())
            });
            let escaped = match aborted {
                Err(ScratchError::Aborted(escaped)) => escaped,
                other => panic!("expected aborted scratch stack, got {other:?}"),
            };
            assert_eq!(escaped.amount(host), 0);
            assert_eq!(
                host.transaction(|host, tx| escaped.insert(host, tx, &buckets, 1)),
                Ok(0)
            );

            // A committed loan accepts same-item inserts up to max stack, refuses different
            // items and fluid-fill item changes, and publishes the updated stack only on commit.
            let committed = host.scratch_stack(bucket.clone(), |host, tx, place| {
                assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                // Another item has nowhere to go: a scratch stack doesn't spill.
                assert_eq!(place.insert(host, tx, &stick, 1), Ok(0));
                assert_eq!(place.amount(host), 3);
                // Filling it would make it a water bucket, another item: refused.
                let block = level.capability(host, &FLUIDS, [2, 64, 0], None).unwrap();
                let held = place.capability(host, &ITEM_FLUIDS).unwrap();
                let filled = move_resources(host, &block, &held, |_| true, 1000, Some(tx));
                assert_eq!(filled, Ok(0));
                // Same-item inserts cap at the item's max stack size (16 for buckets).
                assert_eq!(place.insert(host, tx, &buckets, 20), Ok(13));
                assert_eq!(place.amount(host), 16);
                assert_eq!(place.insert(host, tx, &buckets, 1), Ok(0));
                Ok::<_, ()>(place.clone())
            });
            let (done_place, published) = committed.expect("committed scratch stack");
            assert_eq!(published, Some(api("minecraft:bucket", 16)));
            assert_eq!(done_place.amount(host), 0);
            assert_eq!(
                host.transaction(|host, tx| done_place.insert(host, tx, &buckets, 1)),
                Ok(0)
            );

            // Oversized initial stacks are admitted and preserved (matching `StackItemAccess`
            // and `ItemStacksResourceHandler`), while further inserts cap at max stack (16).
            let mut oversized = bucket.clone();
            oversized.set_count(20).unwrap();
            let over_single = host.scratch_stack(oversized.clone(), |host, tx, place| {
                assert_eq!(place.amount(host), 20);
                assert_eq!(place.insert(host, tx, &buckets, 1), Ok(0));
                assert_eq!(place.extract(host, tx, &buckets, 2), Ok(2));
                assert_eq!(place.amount(host), 18);
                assert_eq!(place.insert(host, tx, &buckets, 1), Ok(0));
                Ok::<_, ()>(())
            });
            assert_eq!(over_single, Ok(((), Some(api("minecraft:bucket", 18)))));

            let over_slots = host.scratch_slots(oversized, |host, tx, place| {
                assert_eq!(place.amount(host), 20);
                assert_eq!(place.extract(host, tx, &buckets, 2), Ok(2));
                assert_eq!(place.insert(host, tx, &buckets, 1), Ok(1));
                assert_eq!(place.amount(host), 18);
                Ok::<_, ()>(())
            });
            assert_eq!(
                over_slots,
                Ok((
                    (),
                    (
                        Some(api("minecraft:bucket", 18)),
                        Some(api("minecraft:bucket", 1)),
                    )
                ))
            );

            // An invalid initial stack (e.g. max_stack_size = 0) fails before the closure runs.
            let mut invalid_max = bucket;
            invalid_max
                .set_components(
                    [(
                        sunrise_core::nbt::JavaString::from("minecraft:max_stack_size"),
                        sunrise_core::nbt::Tag::Int(0),
                    )]
                    .into(),
                )
                .unwrap();
            let invalid = host.scratch_stack(invalid_max, |_, _, _| -> Result<(), ()> {
                panic!("invalid scratch stack must not run closure");
            });
            assert_eq!(
                invalid,
                Err(ScratchError::Stack(StackWriteError::InvalidMaximum))
            );

            assert_eq!(game.world.block(2, 64, 0), cauldron);
            assert_eq!(game.live_scratch_count(), 0);
        });

        // A handle leaked out of one `Game` never aliases a newly created successor `Game`'s
        // scratch loan even if allocator addresses are reused.
        let mut leaked = None;
        with_game(|game| {
            let bucket = ItemStack::new("minecraft:bucket", 1)
                .try_into_api()
                .unwrap();
            let host: &mut dyn ServerHost = game;
            let (place, _) = host
                .scratch_stack(bucket, |_, _, place| Ok::<_, ()>(place.clone()))
                .unwrap();
            leaked = Some(place);
        });
        let leaked = leaked.unwrap();
        with_game(|game| {
            let bucket = ItemStack::new("minecraft:bucket", 4)
                .try_into_api()
                .unwrap();
            let buckets = ItemResource::of(&bucket);
            let host: &mut dyn ServerHost = game;
            let res = host.scratch_stack(bucket.clone(), |host, tx, place| {
                assert_ne!(place, &leaked);
                assert_eq!(leaked.amount(host), 0);
                assert_eq!(leaked.insert(host, tx, &buckets, 1), Ok(0));
                assert_eq!(leaked.extract(host, tx, &buckets, 1), Ok(0));
                assert_eq!(place.amount(host), 4);
                Ok::<_, ()>(())
            });
            assert_eq!(res, Ok(((), Some(bucket))));
            assert_eq!(game.live_scratch_count(), 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_scratch_stack_merges_into_its_parent_and_ends_with_the_outermost() {
        use mod_api::ScratchError;
        use std::panic::{AssertUnwindSafe, catch_unwind};
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[0] = Some(ItemStack::new("minecraft:bucket", 10));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let api = |name, count| ItemStack::new(name, count).try_into_api().unwrap();
            let bucket = api("minecraft:bucket", 1);
            let buckets = ItemResource::of(&bucket);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 0,
            };

            // A nested transaction that commits inside `scratch_stack` stays provisional:
            // aborting the enclosing `scratch_stack` rolls back both the scratch slot and world
            // state, and invalidates any escaped handle.
            {
                let host: &mut dyn ServerHost = game;
                let provisional_abort = host.scratch_stack(bucket.clone(), |host, _, place| {
                    host.transaction(|host, tx| {
                        assert_eq!(slot.extract(host, tx, &buckets, 2), Ok(2));
                        assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                        assert_eq!(place.amount(host), 3);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    assert_eq!(place.amount(host), 3);
                    assert_eq!(slot.amount(host), 8);
                    Err::<(), _>(place.clone())
                });
                let aborted_place = match provisional_abort {
                    Err(ScratchError::Aborted(place)) => place,
                    other => panic!("expected aborted scratch stack, got {other:?}"),
                };
                assert_eq!(aborted_place.amount(host), 0);
                assert_eq!(slot.amount(host), 10);

                // Inside a committing `scratch_stack`: a committed child stays provisional until
                // the loan commits, an aborted child restores the scratch slot BEFORE fallback
                // runs, and committing the loan publishes both the final stack and world writes.
                let committed = host.scratch_stack(bucket.clone(), |host, tx, place| {
                    host.transaction(|host, tx| {
                        assert_eq!(slot.extract(host, tx, &buckets, 1), Ok(1));
                        assert_eq!(place.insert(host, tx, &buckets, 1), Ok(1));
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    assert_eq!(place.amount(host), 2);
                    assert_eq!(slot.amount(host), 9);

                    let aborted = host.transaction(|host, tx| {
                        assert_eq!(slot.extract(host, tx, &buckets, 3), Ok(3));
                        assert_eq!(place.insert(host, tx, &buckets, 3), Ok(3));
                        assert_eq!(place.amount(host), 5);
                        Err::<(), _>(())
                    });
                    assert_eq!(aborted, Err(TransactionError::Aborted(())));
                    assert_eq!(place.amount(host), 2);
                    assert_eq!(slot.amount(host), 9);

                    // Fallback sees the restored scratch slot and commits with the loan.
                    assert_eq!(slot.extract(host, tx, &buckets, 2), Ok(2));
                    assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                    assert_eq!(place.amount(host), 4);
                    Ok::<_, ()>(())
                });
                assert_eq!(committed, Ok(((), Some(api("minecraft:bucket", 4)))));
                assert_eq!(slot.amount(host), 7);

                // A `scratch_stack` nested inside a longer-lived outer `host.transaction`
                // invalidates its escaped handle immediately on exit while the outer transaction
                // is still open.
                let outer_host = host.transaction(|host, outer_tx| {
                    let (escaped, published) = host
                        .scratch_stack(bucket.clone(), |host, tx, place| {
                            assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                            Ok::<_, ()>(place.clone())
                        })
                        .unwrap();
                    assert_eq!(published, Some(api("minecraft:bucket", 3)));
                    assert_eq!(escaped.amount(host), 0);
                    assert_eq!(escaped.insert(host, outer_tx, &buckets, 1), Ok(0));
                    Ok::<_, ()>(())
                });
                assert_eq!(outer_host, Ok(()));
            }

            // On the real `Game` journal, a nested `scratch_stack` inside a longer-lived outer
            // transaction (whether it commits or aborts) leaves zero live scratch slots and zero
            // `scratch_kept` entries on the outer frame while that outer frame is still open.
            let outer_journal = game.storage_transaction(|game, outer_tx| {
                let (escaped, published) = game
                    .scratch_stack(bucket.clone(), |host, tx, place| {
                        host.transaction(|host, inner_tx| {
                            assert_eq!(place.insert(host, inner_tx, &buckets, 2), Ok(2));
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        assert_eq!(place.insert(host, tx, &buckets, 1), Ok(1));
                        Ok::<_, ()>(place.clone())
                    })
                    .unwrap();
                assert_eq!(published, Some(api("minecraft:bucket", 4)));
                assert_eq!(escaped.amount(game), 0);
                assert_eq!(escaped.insert(game, outer_tx, &buckets, 1), Ok(0));
                assert_eq!(game.live_scratch_count(), 0);
                assert_eq!(game.current_scratch_kept_count(), 0);

                let aborted = game.scratch_stack(bucket.clone(), |host, tx, place| {
                    assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                    Err::<(), _>(place.clone())
                });
                let aborted_escaped = match aborted {
                    Err(ScratchError::Aborted(place)) => place,
                    other => panic!("expected aborted nested scratch stack, got {other:?}"),
                };
                assert_eq!(aborted_escaped.amount(game), 0);
                assert_eq!(aborted_escaped.insert(game, outer_tx, &buckets, 1), Ok(0));
                assert_eq!(game.live_scratch_count(), 0);
                assert_eq!(game.current_scratch_kept_count(), 0);
                Ok::<_, ()>(())
            });
            assert_eq!(outer_journal, Ok(()));

            // A panic unwind inside `scratch_stack` restores world state and cleans up the live
            // scratch slot with zero leak.
            let before_panic = slot.amount(game);
            let mut escaped_from_panic = None;
            let panicked = catch_unwind(AssertUnwindSafe(|| {
                let _ = game.scratch_stack(bucket, |host, tx, place| -> Result<(), ()> {
                    escaped_from_panic = Some(place.clone());
                    assert_eq!(slot.extract(host, tx, &buckets, 2), Ok(2));
                    assert_eq!(place.insert(host, tx, &buckets, 2), Ok(2));
                    assert_eq!(place.amount(host), 3);
                    panic!("broken scratch callback");
                });
            }));
            assert!(panicked.is_err());
            assert_eq!(slot.amount(game), before_panic);
            assert_eq!(game.live_scratch_count(), 0);
            assert_eq!(game.current_scratch_kept_count(), 0);
            assert!(!game.has_open_transaction());
            let escaped_from_panic = escaped_from_panic.expect("captured place before panic");
            assert_eq!(escaped_from_panic.amount(game), 0);
            assert_eq!(
                game.transaction(|host, tx| escaped_from_panic.insert(host, tx, &buckets, 1)),
                Ok(0)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_strict_slot_refuses_what_the_plain_slot_spreads() {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let slots = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let Some(BlockEntity::Chest(items)) =
                    game.world.block_entities.get_mut(&(0, 64, 0))
                else {
                    panic!("no chest")
                };
                items[..2].to_vec()
            };
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 0))
            {
                items[0] = Some(ItemStack::new("minecraft:stick", 64));
            }
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let place = ItemPlace::BlockEntity {
                level,
                pos: [0, 64, 0],
                side: None,
                slot: 0,
            };
            let strict = ItemPlace::Strict(Box::new(place.clone()));
            let host: &mut dyn ServerHost = game;
            let went = |host: &mut dyn ServerHost, at: &ItemPlace| {
                host.transaction(|host, tx| at.insert(host, tx, &stick, 5))
            };
            assert_eq!(went(&mut *host, &strict), Ok(0));
            assert_eq!(went(&mut *host, &place), Ok(5));
            assert_eq!(
                slots(game),
                [
                    Some(ItemStack::new("minecraft:stick", 64)),
                    Some(ItemStack::new("minecraft:stick", 5))
                ]
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn one_by_one_shows_one_bucket_of_sixteen_and_fills_one() {
        use mod_api::{FLUIDS, ITEM_FLUIDS, resource_util::move_resources};
        with_game(|game| {
            let level = with_cauldron(game);
            let mut player = guest(1);
            player.inventory[0] = Some(ItemStack::new("minecraft:bucket", 16));
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 0,
            };
            let one = ItemPlace::OneByOne(Box::new(slot.clone()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                one.with_stack(host, |stack| (
                    stack.count(),
                    stack.project().map(|s| s.count())
                )),
                Some((1, Some(1)))
            );
            assert_eq!(slot.amount(host), 16);
            let block = level.capability(host, &FLUIDS, [2, 64, 0], None).unwrap();
            let bucket = one.capability(host, &ITEM_FLUIDS).unwrap();
            let moved = host.transaction(|host, tx| {
                move_resources(host, &block, &bucket, |_| true, 1000, Some(tx))
            });
            assert_eq!(moved, Ok(1000));
            let held = game.server.lan.as_ref().unwrap().hub.guests[0]
                .inventory
                .clone();
            assert_eq!(held[0], Some(ItemStack::new("minecraft:bucket", 15)));
            let water = Some(ItemStack::new("minecraft:water_bucket", 1));
            assert!(held.contains(&water));
        });
    }
}
