#[cfg(any(test, feature = "test-support"))]
use sunrise_gameplay_common::game::Mining;

#[cfg(any(test, feature = "test-support"))]
impl crate::game::server_side::lan_actions_server::GuestPlay {
    #[cfg(any(test, feature = "test-support"))]
    pub fn arm_mining_for_test(
        &mut self,
        pos: [i32; 3],
        start: u64,
        delayed: Option<([i32; 3], u64)>,
    ) {
        self.digging = Some((pos, [0, 1, 0]));
        self.dig_start = start;
        self.mining = Some(Mining {
            pos,
            progress: 0.0,
            per_tick: 0.0,
            ticks: 1,
        });
        self.delayed = delayed;
        self.last_progress = -1;
    }
}

#[cfg(test)]
pub mod tests {
    use sunrise_gameplay_common::game::lan_actions::{eye_columns, reset_use_attack_strength};
    use sunrise_sim::pose::Pose;

    use super::stop_menu::StopMenu;

    #[test]
    fn use_swing_resets_attack_charge_only_when_started() {
        let mut ticker = 20;
        reset_use_attack_strength(&mut ticker, 3, 4);
        // A sword's attack speed is 1.6: vanilla's (ticker + 0.5) / 12.5.
        assert_eq!((ticker as f32 + 0.5) / 12.5, 0.04);

        ticker = 8;
        reset_use_attack_strength(&mut ticker, 4, 4);
        assert_eq!(ticker, 8); // No swing, or a swing rejected in its first half.
    }

    #[test]
    #[ignore = "daily"]
    #[cfg(any())]
    fn menu_rules_match_guest_packets_and_prediction() {
        use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
        use std::sync::Arc;
        use sunrise_items::items::ItemStack;
        use sunrise_level::{
            chunk::Chunk,
            containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModMenuType},
        };
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        use sunrise_sim::{
            lan::menu::Slots,
            menu::{Input, Kind, Menu},
        };

        struct Part;
        #[mod_api::sunrise_part]
        impl MenuPart for Part {
            fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
                if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                    == "minecraft:granite"
                    && event.stack.count() < 8
                {
                    event.may_place = false;
                }
                if event.slot == 0 {
                    if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                        == "minecraft:diorite"
                        && event.stack.count() >= 8
                    {
                        event.max_stack = 3;
                    }
                    if matches!(
                        mod_api::__private::item_key_id(event.stack.item()).unwrap(),
                        "minecraft:andesite" | "minecraft:stone"
                    ) {
                        event.max_stack = 3;
                    }
                    if mod_api::__private::item_key_id(event.stack.item()).unwrap()
                        == "minecraft:dirt"
                    {
                        event.may_place = false;
                    }
                }
            }
        }
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.menu_part("minecraft:generic_9x3", Part);
                r.menu_part("test:menu", Part);
            }
        }
        let mods: Mods = vec![ModEntry::new("menu_rules", Box::new(Mod))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let pos = (1, 64, 1);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                let menu_type = Box::leak(Box::new(ModMenuType {
                    id: "test:menu",
                    owner: "test",
                    slots: 27,
                    player_inventory: true,
                    data_slots: 0,
                    owned_slots: None,
                    results: Vec::new(),
                    inputs: Vec::new(),
                    crafting: None,
                    links: None,
                }));
                for is_mod in [false, true] {
                    game.world.block_entities.insert(
                        pos,
                        if is_mod {
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:block".into(),
                                ModBlockEntityFields {
                                    items: vec![None; 27],
                                    ..Default::default()
                                },
                            )))
                        } else {
                            BlockEntity::Chest(vec![None; 27])
                        },
                    );
                    let host = game.server.lan.as_mut().unwrap();
                    host.hub.open_container(
                        12,
                        "minecraft:generic_9x3",
                        sunrise_core::nbt::Tag::String("test".into()),
                        pos,
                        Slots::Block(pos),
                        vec![None; 27],
                        None,
                    );
                    host.hub.guests.update(0, |guest| {
                        let opened = guest.menu.as_mut().unwrap();
                        if is_mod {
                            opened.registry_id = "test:menu".into();
                            opened.mod_type = Some(menu_type);
                        }
                    });
                    let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                    for (item, existing, expected) in [
                        ("minecraft:dirt", 0, 0),
                        ("minecraft:stone", 0, 3),
                        ("minecraft:diorite", 2, 3),
                    ] {
                        let stack = ItemStack::new(item, 8);
                        game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                            (existing > 0).then(|| ItemStack::new(item, existing));
                        let mut prediction = Menu::new(if is_mod {
                            Kind::Mod(menu_type)
                        } else {
                            Kind::Chest(3)
                        });
                        prediction.set_registry_id(Some(if is_mod {
                            "test:menu"
                        } else {
                            "minecraft:generic_9x3"
                        }));
                        prediction
                            .load_slot(0, (existing > 0).then(|| ItemStack::new(item, existing)));
                        prediction.carried = Some(stack.clone());
                        prediction
                            .click(&game.menu_env(), 0, 0, Input::Pickup)
                            .unwrap();
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| guest.carried = Some(stack));
                        game.lan_guest_packet(
                            12,
                            Serverbound::ContainerClick(ContainerClick {
                                container: window,
                                state: 0,
                                slot: 0,
                                button: 0,
                                input: 0,
                                changed: Vec::new(),
                                carried: None,
                            }),
                        );
                        let actual = game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .map_or(0, |s| s.count);
                        assert_eq!(actual, expected, "{item} mod={is_mod}");
                        assert_eq!(prediction.slot(0).map_or(0, |s| s.count), actual);
                        assert_eq!(
                            actual + prediction.carried.as_ref().map_or(0, |s| s.count),
                            existing + 8
                        );
                        assert_eq!(
                            prediction.carried.as_ref().map_or(0, |s| s.count),
                            game.server.lan.as_ref().unwrap().hub.guests[0]
                                .carried
                                .as_ref()
                                .map_or(0, |s| s.count)
                        );
                    }
                    for (item, existing, second, carried, creative, expected) in [
                        ("minecraft:granite", 0, 0, 8, false, (4, 4, 0)),
                        ("minecraft:andesite", 8, 0, 8, false, (3, 4, 9)),
                        ("minecraft:stone", 2, 0, 64, true, (3, 64, 0)),
                        ("minecraft:stone", 2, 63, 64, true, (3, 64, 62)),
                    ] {
                        let slots = game.world.block_entities.get_mut(&pos).unwrap().slots_mut();
                        slots[0] = (existing > 0).then(|| ItemStack::new(item, existing));
                        slots[1] = (second > 0).then(|| ItemStack::new(item, second));
                        let mut prediction = Menu::new(if is_mod {
                            Kind::Mod(menu_type)
                        } else {
                            Kind::Chest(3)
                        });
                        prediction.set_registry_id(Some(if is_mod {
                            "test:menu"
                        } else {
                            "minecraft:generic_9x3"
                        }));
                        prediction.load_slot(0, slots[0].clone());
                        prediction.load_slot(1, slots[1].clone());
                        prediction.carried =
                            Some(sunrise_items::items::ItemStack::new(item, carried));
                        prediction.creative = creative;
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| {
                                guest.carried =
                                    Some(sunrise_items::items::ItemStack::new(item, carried));
                                guest.game_mode = if creative {
                                    sunrise_sim::lan::guest::CREATIVE
                                } else {
                                    0
                                };
                            });
                        let kind = if creative { 2 << 2 } else { 0 };
                        for (slot, button) in
                            [(-999, kind), (0, kind | 1), (1, kind | 1), (-999, kind | 2)]
                        {
                            prediction
                                .click(&game.menu_env(), slot, button, Input::QuickCraft)
                                .unwrap();
                            game.lan_guest_packet(
                                12,
                                Serverbound::ContainerClick(ContainerClick {
                                    container: window,
                                    state: 0,
                                    slot: slot as i16,
                                    button: button as i8,
                                    input: 5,
                                    changed: Vec::new(),
                                    carried: None,
                                }),
                            );
                        }
                        let slots = game.world.block_entities.get(&pos).unwrap().slots();
                        let result = (
                            slots[0].as_ref().map_or(0, |s| s.count),
                            slots[1].as_ref().map_or(0, |s| s.count),
                            game.server.lan.as_ref().unwrap().hub.guests[0]
                                .carried
                                .as_ref()
                                .map_or(0, |s| s.count),
                        );
                        let predicted = (
                            prediction.slot(0).map_or(0, |s| s.count),
                            prediction.slot(1).map_or(0, |s| s.count),
                            prediction.carried.as_ref().map_or(0, |s| s.count),
                        );
                        assert_eq!(result, expected, "{item} mod={is_mod}");
                        assert_eq!(predicted, expected, "{item} mod={is_mod}");
                        if !creative {
                            assert_eq!(result.0 + result.1 + result.2, existing + carried);
                        }
                    }
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.game_mode = 0);
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 2));
                    let mut prediction = Menu::new(if is_mod {
                        Kind::Mod(menu_type)
                    } else {
                        Kind::Chest(3)
                    });
                    prediction.set_registry_id(Some(if is_mod {
                        "test:menu"
                    } else {
                        "minecraft:generic_9x3"
                    }));
                    prediction.load_slot(
                        0,
                        Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 2)),
                    );
                    prediction.set_player_item(
                        9,
                        Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 4)),
                    );
                    prediction
                        .click(&game.menu_env(), 27, 0, Input::QuickMove)
                        .unwrap();
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory[9] =
                                Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 4))
                        });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 27,
                            button: 0,
                            input: 1,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        2
                    );
                    assert_eq!(prediction.slot(0).unwrap().count, 2);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory.fill(None);
                            guest.inventory[0] =
                                Some(sunrise_items::items::ItemStack::new("minecraft:stone", 8));
                        });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 54,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 8);
                    assert_eq!(guest.inventory.iter().flatten().count(), 1);
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(sunrise_items::items::ItemStack::new("minecraft:dirt", 2));
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory.fill(None);
                            guest.inventory[0] =
                                Some(sunrise_items::items::ItemStack::new("minecraft:stone", 8));
                        });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(
                        game.world.block_entities.get(&pos).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        3
                    );
                    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[0].as_ref().unwrap().count, 5);
                    assert_eq!(guest.inventory[1].as_ref().unwrap().count, 2);
                    // A component-limited displaced stack fills to 16, then leaves 7 elsewhere.
                    let mut limited = ItemStack::new("minecraft:dirt", 8);
                    limited.data_mut().set_other(
                        "minecraft:max_stack_size",
                        Some(sunrise_core::nbt::Tag::Int(16)),
                    );
                    game.world.block_entities.get_mut(&pos).unwrap().slots_mut()[0] =
                        Some(limited.clone());
                    limited.count = 15;
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory.fill(None);
                            guest.inventory[1] = Some(limited);
                            guest.selected = 1;
                            guest.inventory[0] =
                                Some(sunrise_items::items::ItemStack::new("minecraft:stone", 8));
                        });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 2,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.inventory[1].as_ref().unwrap().count, 16);
                    assert_eq!(
                        guest
                            .inventory
                            .iter()
                            .flatten()
                            .filter(|s| &*s.item == "minecraft:dirt")
                            .map(|s| s.count)
                            .sum::<u32>(),
                        23
                    );
                    game.guest_close_all(12);
                }
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn crafting_guest_self_swap_keeps_eight_items() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let table = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafting_table")
                .unwrap();
            game.world.set(1, 64, 1, table).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| {
                    guest.inventory[0] =
                        Some(sunrise_items::items::ItemStack::new("minecraft:stone", 8));
                });
            game.lan_open_crafting(12, [1, 64, 1]);
            let window = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .menus
                .get(&12)
                .unwrap()
                .crafting
                .table
                .as_ref()
                .unwrap()
                .0;
            game.lan_guest_packet(
                12,
                Serverbound::ContainerClick(ContainerClick {
                    container: window,
                    state: 0,
                    slot: 37,
                    button: 0,
                    input: 2,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.inventory[0].as_ref().unwrap().count, 8);
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum::<u32>(),
                8
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_menu_handle_and_use_each_count_the_guest_once() {
        use crate::game::sculk_server::TEST_POSTED_EVENTS;
        use mod_api::{BlockUpdateFlags, GameMode};
        use std::sync::Arc;
        use sunrise_gameplay_common::game::{
            Hit, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
        };
        use sunrise_level::chunk::Chunk;
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let pos = (3, 64, 4);
            let at = [pos.0, pos.1, pos.2];
            let hit = Hit {
                pos: at,
                inside: false,
                face: [0, 1, 0],
                point: [3.5, 65.0, 4.5],
            };
            let use_menu = |game: &mut crate::game::ServerGame<'_>| {
                assert_eq!(
                    game.as_guest(12, |game| game.use_hand(Some(&hit)).unwrap()),
                    Some(true)
                );
            };
            let take_signals = |game: &mut crate::game::ServerGame<'_>| {
                let lids = std::mem::take(&mut game.server.lan_block_events)
                    .into_iter()
                    .filter_map(|(p, id, count, _)| (p == at && id == 1).then_some(count))
                    .collect::<Vec<_>>();
                let sounds = std::mem::take(&mut game.server.pending_sounds)
                    .into_iter()
                    .map(|sound| sound.event)
                    .collect::<Vec<_>>();
                let events = TEST_POSTED_EVENTS.with(|posted| {
                    std::mem::take(posted.borrow_mut().as_mut().unwrap())
                        .into_iter()
                        .map(|(event, _, _)| event)
                        .collect::<Vec<_>>()
                });
                (lids, sounds, events)
            };
            for block in ["chest", "ender_chest", "shulker_box"] {
                game.stop_audio_capture();
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state(&format!("minecraft:{block}"))
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, at, state, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.start_audio_capture();
                TEST_POSTED_EVENTS.with(|posted| *posted.borrow_mut() = Some(Vec::new()));
                let count = |game: &crate::game::ServerGame<'_>| {
                    if block == "shulker_box" {
                        game.server
                            .be_anims
                            .shulkers
                            .get(&at)
                            .map_or(0, |s| s.open_count)
                    } else {
                        game.server.lid_openers.get(&pos).copied().unwrap_or(0)
                    }
                };
                // A shulker's veto can change the acting player to spectator, but
                // the use still spends the click and must never touch its opener count.
                let switch = (block == "shulker_box").then_some(GameMode::Spectator);
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "stop_menu",
                        Box::new(StopMenu(switch)),
                    )]
                    .into(),
                    || {
                        use_menu(game);
                        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                        assert!(guest.menu.is_none());
                        assert_eq!(guest.spectator(), switch.is_some());
                        assert_eq!(count(game), 0, "{block}");
                        assert_eq!(take_signals(game), (vec![], vec![], vec![]), "{block}");
                    },
                );
                if switch.is_some() {
                    // In the reverse direction the rejected spectator must not
                    // remove the other player's actual opener.
                    assert_eq!(
                        game.open_block_menu(13, pos),
                        sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                    );
                    assert_eq!(count(game), 1);
                    take_signals(game);
                    mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new(
                            "stop_menu",
                            Box::new(StopMenu(Some(GameMode::Survival))),
                        )]
                        .into(),
                        || {
                            assert_eq!(
                                game.as_guest(12, |game| {
                                    game.spectator_open_menu(at);
                                    game.container == Some(pos)
                                }),
                                Some(true)
                            );
                            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                            assert!(!guest.spectator());
                            assert!(guest.menu.is_none());
                            assert_eq!(count(game), 1);
                            assert_eq!(take_signals(game), (vec![], vec![], vec![]));
                        },
                    );
                    game.guest_close_all(13);
                    assert_eq!(count(game), 0);
                    take_signals(game);
                }
                for direct in [false, true] {
                    if direct {
                        assert_eq!(
                            game.open_block_menu(12, pos),
                            sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                        );
                    } else {
                        use_menu(game);
                    }
                    assert!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .is_some()
                    );
                    assert_eq!(count(game), 1, "{block}");
                    assert_eq!(
                        take_signals(game),
                        (
                            vec![1],
                            vec![format!("minecraft:block.{block}.open")],
                            vec!["minecraft:container_open"],
                        ),
                        "{block}"
                    );
                    game.guest_close_all(12);
                    assert_eq!(count(game), 0, "{block}");
                    assert_eq!(
                        take_signals(game),
                        (
                            vec![0],
                            vec![format!("minecraft:block.{block}.close")],
                            vec!["minecraft:container_close"],
                        ),
                        "{block}"
                    );
                }
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.game_mode = 3;
                    });
                assert_eq!(
                    game.open_block_menu(12, pos),
                    sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                );
                assert_eq!(count(game), 0, "{block}");
                game.guest_close_all(12);
                assert_eq!(take_signals(game), (vec![], vec![], vec![]), "{block}");
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.game_mode = 0;
                    });
            }
            TEST_POSTED_EVENTS.with(|posted| *posted.borrow_mut() = None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_part_use_item_on_consumes_once() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::InteractionResult as Answer;
        use mod_api::{self, BlockPart, Hooks, HostSide, Registrar, UseItemOnEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering::SeqCst},
        };
        use sunrise_level::chunk::Chunk;

        #[derive(Clone)]
        struct BlockUse(Arc<AtomicUsize>);
        #[mod_api::sunrise_part]
        impl BlockPart for BlockUse {
            fn use_item_on(&self, _: HostSide<'_>, _: &mut UseItemOnEvent<'_>) -> Option<Answer> {
                self.0.fetch_add(1, SeqCst);
                Some(Answer::Success)
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for BlockUse {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:lever");
                r.block_part(block_key, self.clone());
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "block_use",
            Box::new(BlockUse(calls.clone())),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 4, stone).unwrap();
                game.world.set(0, 64, 4, lever).unwrap();
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.world.block(0, 64, 4), lever);
                assert_eq!(calls.load(SeqCst), 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn replacing_empty_note_block_use_does_not_award_vanilla_stats() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::InteractionResult as Answer;
        use mod_api::{self, BlockPart, Hooks, HostSide, Registrar, UseWithoutItemEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering::SeqCst},
        };
        use sunrise_level::chunk::Chunk;

        #[derive(Clone)]
        struct Replacement(Arc<AtomicBool>);
        #[mod_api::sunrise_part]
        impl BlockPart for Replacement {
            fn use_without_item(
                &self,
                _: HostSide<'_>,
                _: &mut UseWithoutItemEvent<'_>,
            ) -> Option<Answer> {
                self.0.load(SeqCst).then_some(Answer::Success)
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for Replacement {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let block_key = r.block_key("minecraft:note_block");
                r.block_part(block_key, self.clone());
            }
        }
        let replace = Arc::new(AtomicBool::new(true));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "empty_note_replacement",
            Box::new(Replacement(replace.clone())),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let note = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:note_block")
                    .unwrap();
                game.world.set(0, 64, 4, note).unwrap();
                let hit = sunrise_gameplay_common::game::Hit {
                    pos: [0, 64, 4],
                    face: [0, 1, 0],
                    point: [0.5, 65.0, 4.5],
                    inside: false,
                };
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.world.block(0, 64, 4), note);
                assert_eq!(game.stats.get("custom", "minecraft:tune_noteblock"), 0);

                // Delegating still runs the actual method and awards its stat.
                replace.store(false, SeqCst);
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_ne!(game.world.block(0, 64, 4), note);
                assert_eq!(game.stats.get("custom", "minecraft:tune_noteblock"), 1);
            });
        });
    }

    /// #4124: a STOP is measured from the START that named its block (`destroyPos`) — an abort, or
    /// a STOP from out of reach, ends the dig the guest's button held but leaves that clock — and
    /// breaks the block at 70 % of the dig, or the server finishes it alone from the same START.
    /// A guest that reaches a block only at a jump's apex keeps its dig this way, and no other
    /// block or new body inherits the clock.
    #[test]
    fn a_stop_after_a_fall_is_measured_from_the_starts_clock() {
        use crate::game::server_side::lan_actions_server::{GuestPlay, StopDig, stop_dig};
        use sunrise_gameplay_common::game::Mining;
        let pos = [12, 70, -5];
        let mut play = GuestPlay {
            digging: Some((pos, [0, 1, 0])),
            mining: Some(Mining {
                pos,
                progress: 0.5,
                per_tick: 0.0,
                ticks: 8,
            }),
            destroy_pos: Some(pos),
            dig_start: 1000,
            ..GuestPlay::default()
        };
        // The landing: the dig the button held is gone, the clock the START set is not.
        play.dig_ended();
        assert!(play.digging.is_none() && play.mining.is_none());
        // 13 ticks of 0.05: 65 % of the dig, so the server finishes it from tick 1000 itself.
        assert_eq!(
            stop_dig(&play, pos, false, 1012, 0.05),
            StopDig::Delay(1000)
        );
        // 16 ticks: 80 %, the STOP breaks the block outright.
        assert_eq!(stop_dig(&play, pos, false, 1015, 0.05), StopDig::Break);
        // A block the START did not name never gets the clock, and a broken one only its crack.
        assert_eq!(
            stop_dig(&play, [12, 71, -5], false, 1015, 0.05),
            StopDig::Other
        );
        assert_eq!(stop_dig(&play, pos, true, 1015, 0.05), StopDig::Gone);
        // A respawn's new ServerPlayerGameMode starts from no clock at all.
        play.destroy_pos = None;
        assert_eq!(stop_dig(&play, pos, false, 1015, 0.05), StopDig::Other);
    }

    /// #3684: a player shrunk to `minecraft:scale` 0.1 who stands 0.05 from a
    /// wall has eyes 0.048 wide, which don't reach into it: they don't
    /// suffocate there, as a full-sized player 0.3 from the wall doesn't.
    #[test]
    fn a_small_player_checks_only_the_blocks_at_their_small_eyes() {
        let at = [205.95, 100.0, 530.5];
        assert_eq!(
            eye_columns(at, Pose::Standing.dimensions(0.1)),
            [205..=205, 530..=530]
        );
        assert_eq!(
            eye_columns(at, Pose::Standing.dimensions(1.0)),
            [205..=206, 530..=530]
        );
        assert_eq!(
            eye_columns([205.7, 100.0, 530.5], Pose::Standing.dimensions(1.0)),
            [205..=205, 530..=530]
        );
    }
}

#[cfg(test)]
pub(in crate::game) mod hotbar_tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{
        Flow, Hooks, ModEntry, PlayerSwitchHotbarSlotPostEvent, PlayerSwitchHotbarSlotPreEvent,
        ServerHost,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_protocol::packets::play;
    use {
        crate::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    /// What the hooks heard, as (pre or post, uuid, entity id, old, new).
    type Heard = Arc<Mutex<Vec<(&'static str, u128, i32, i32, i32)>>>;

    /// Hears both hooks and refuses a switch to slot 8.
    struct Hotbar(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Hotbar {
        fn player_switch_hotbar_slot_pre(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerSwitchHotbarSlotPreEvent,
        ) -> Flow {
            let row = ("pre", e.player, e.player_entity_id, e.old_slot, e.new_slot);
            self.0.lock().unwrap().push(row);
            if e.new_slot == 8 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn player_switch_hotbar_slot_post(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerSwitchHotbarSlotPostEvent,
        ) {
            let row = ("post", e.player, e.player_entity_id, e.old_slot, e.new_slot);
            self.0.lock().unwrap().push(row);
        }
    }

    /// `PlayerSwitchHotbarSlotEvent`: pre, then post once the slot changed, both naming the
    /// player by UUID and entity id; the same slot again fires neither, and a cancelled pre
    /// keeps the old slot with no post.
    #[test]
    #[ignore = "daily"]
    fn a_hotbar_switch_fires_pre_then_post_and_pre_can_keep_the_slot() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("hotbar", Box::new(Hotbar(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.server.lan.as_mut().unwrap().hub.guests.push(guest(1));
                let switch = |game: &mut crate::game::ServerGame<'_>, slot| {
                    game.lan_guest_packet(
                        1,
                        play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot }),
                    );
                    game.server.lan.as_ref().unwrap().hub.guests[0].selected
                };
                assert_eq!(switch(game, 3), 3);
                assert_eq!(switch(game, 3), 3);
                assert_eq!(switch(game, 8), 3);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("pre", 123, 20, 0, 3),
                        ("post", 123, 20, 0, 3),
                        ("pre", 123, 20, 3, 8),
                    ]
                );
            });
        });
    }
}

#[cfg(test)]
pub(in crate::game) mod respawn_position_tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Hooks, LevelDimension, ModEntry, PlayerRespawnPositionEvent, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    /// What the hook heard: the player's UUID, the original level and point,
    /// and whether the block was missing.
    type Heard = Arc<Mutex<Vec<(u128, LevelDimension, [f64; 3], bool)>>>;

    /// Moves every respawn ten blocks east, turns the player north, and
    /// keeps no respawn point for next time.
    struct East(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for East {
        fn player_respawn_position(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerRespawnPositionEvent,
        ) {
            let row = (
                e.player.uuid(),
                e.original_level,
                e.original_position,
                e.missing_respawn_block,
            );
            self.0.lock().unwrap().push(row);
            e.position[0] += 10.0;
            e.yaw = 180.0;
            e.copy_original_spawn_position = false;
        }
    }

    /// `PlayerRespawnPositionEvent` on a server: a guest whose forced point
    /// needs no block respawns there, the mod hears that point with the
    /// player's handle, and its new point, facing and copy choice are what
    /// the player gets.
    #[test]
    #[ignore = "daily"]
    fn a_mod_moves_a_respawn_and_drops_the_point() {
        let heard = Heard::default();
        let mods = vec![ModEntry::new("east", Box::new(East(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut player = guest(1);
                player.dead = true;
                player.spawn = Some([5.5, 64.0, 5.5]);
                game.server.lan.as_mut().unwrap().hub.guests.push(player);
                game.guest_respawn(1);
                let heard = heard.lock().unwrap();
                let [(uuid, level, original, missing)] = heard[..] else {
                    panic!("heard {heard:?}");
                };
                assert_eq!(
                    (uuid, level, missing),
                    (123, LevelDimension::Overworld, false)
                );
                let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert!(!g.dead);
                assert_eq!(g.position, [original[0] + 10.0, original[1], original[2]]);
                assert_eq!(g.yaw, 180.0);
                assert_eq!((g.spawn, &g.respawn_elsewhere), (None, &None));
            });
        });
    }

    /// Hears each respawn's facing and changes nothing.
    struct Listen(Arc<Mutex<Vec<([f64; 3], f32, f32)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn player_respawn_position(
            &self,
            _: &mut dyn ServerHost,
            e: &mut PlayerRespawnPositionEvent,
        ) {
            self.0.lock().unwrap().push((*e.position, e.yaw, e.pitch));
        }
    }

    /// A mod that only listens changes nothing: the guest respawns at its
    /// point facing the respawn direction, not the way it died, and keeps
    /// the point.
    #[test]
    #[ignore = "daily"]
    fn a_listening_mod_leaves_the_respawn_as_it_was() {
        let heard = Arc::default();
        let mods = vec![ModEntry::new(
            "listen",
            Box::new(Listen(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut player = guest(1);
                player.dead = true;
                (player.yaw, player.pitch) = (90.0, 30.0);
                player.spawn = Some([5.5, 64.0, 5.5]);
                game.server.lan.as_mut().unwrap().hub.guests.push(player);
                game.guest_respawn(1);
                let heard = heard.lock().unwrap();
                let [(found, yaw, pitch)] = heard[..] else {
                    panic!("heard {heard:?}");
                };
                assert_eq!((yaw, pitch), (0.0, 0.0), "told the respawn facing");
                let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert!(!g.dead);
                assert_eq!(g.position, found);
                assert_eq!((g.yaw, g.pitch), (0.0, 0.0));
                assert_eq!(g.spawn, Some([5.5, 64.0, 5.5]));
            });
        });
    }
}

#[cfg(test)]
pub(in crate::game) mod pickup_xp_tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{
        ExperienceOrbWriteError, Flow, Hooks, LevelReadError, ModEntry, PickupXpEvent, ServerHost,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_sim::entities::ExperienceOrb;
    use sunrise_sim::entity_nbt::{orb_to_nbt, orbs_from_nbt};
    use {
        crate::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    /// Listens to `pickup_xp`, canceling it when told to.
    struct Listen(bool);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn pickup_xp(&self, _: &mut dyn ServerHost, _: &mut PickupXpEvent<'_>) -> Flow {
            if self.0 { Flow::Cancel } else { Flow::Continue }
        }
    }

    struct Edit {
        value: Option<i32>,
        count: Option<u32>,
        expect_count_err: Option<ExperienceOrbWriteError>,
        kill: bool,
        cancel: bool,
        calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Edit {
        fn pickup_xp(&self, host: &mut dyn ServerHost, event: &mut PickupXpEvent<'_>) -> Flow {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(event.orb.entity().id(), 40);
            assert_eq!(host.experience_orb(event.orb.entity()), Ok(Some(event.orb)));
            if let Some(value) = self.value {
                assert_eq!(event.orb.set_value(host, value), Ok(()));
                assert_eq!(event.orb.value(host), Ok(value));
            }
            if let Some(count) = self.count {
                match self.expect_count_err {
                    Some(err) => assert_eq!(event.orb.set_count(host, count), Err(err)),
                    None => {
                        assert_eq!(event.orb.set_count(host, count), Ok(()));
                        assert_eq!(event.orb.count(host), Ok(count));
                    }
                }
            }
            if self.kill {
                assert_eq!(event.orb.entity().kill(host), Ok(true));
                assert_eq!(event.orb.count(host), Err(LevelReadError::Stale));
            }
            if self.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    struct ObserveAndEdit {
        seen: Arc<Mutex<Vec<(i32, u32)>>>,
        write_value: Option<i32>,
        write_count: Option<u32>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ObserveAndEdit {
        fn pickup_xp(&self, host: &mut dyn ServerHost, event: &mut PickupXpEvent<'_>) -> Flow {
            let value = event.orb.value(host).expect("live orb value");
            let count = event.orb.count(host).expect("live orb count");
            self.seen.lock().unwrap().push((value, count));
            if let Some(v) = self.write_value {
                assert_eq!(event.orb.set_value(host, v), Ok(()));
            }
            if let Some(c) = self.write_count {
                assert_eq!(event.orb.set_count(host, c), Ok(()));
            }
            Flow::Continue
        }
    }

    fn make_orb(value: i32, count: u32) -> ExperienceOrb {
        let p = Vec3::new(0.5, 70.0, 0.5);
        ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            id: 40,
            position: p,
            previous: p,
            velocity: Vec3::ZERO,
            value,
            age: 0,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            count,
            fire_ticks: 0,
            ticks: 0,
            air: 300,
            health: 5,
            no_gravity: true,
            following: None,
            fall_distance: 0.0,
        }
    }

    /// A guest takes the orb it stands in: the orb's id left in the level and the
    /// guest's total experience after.
    fn take_orb() -> (bool, i32) {
        let mut out = (false, 0);
        with_game(|game| {
            open_to_lan(game);
            let mut g = guest(1);
            g.loaded = true;
            g.position = [0.5, 70.0, 0.5];
            game.server.lan.as_mut().unwrap().hub.guests.push(g);
            game.orbs.push(make_orb(7, 1));
            game.guest_orbs(0);
            let left = game.orbs.iter().any(|o| o.id == 40);
            out = (
                left,
                game.server.lan.as_ref().unwrap().hub.guests[0].xp_total,
            );
        });
        out
    }

    fn run_pickup_mods(
        initial_value: i32,
        initial_count: u32,
        initial_xp: i32,
        mods: Vec<ModEntry>,
    ) -> (Option<(i32, u32)>, i32, u32, Option<Tag>) {
        let mut out = (None, 0, 0, None);
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                open_to_lan(game);
                let mut g = guest(1);
                g.loaded = true;
                g.position = [0.5, 70.0, 0.5];
                g.xp_total = initial_xp;
                game.server.lan.as_mut().unwrap().hub.guests.push(g);
                game.orbs.push(make_orb(initial_value, initial_count));
                game.guest_orbs(0);
                let surviving = game.orbs.iter().find(|o| o.id == 40);
                let saved = surviving.map(|o| orb_to_nbt(o, [0; 4]));
                let state = surviving.map(|o| (o.value, o.count));
                let host = game.server.lan.as_ref().unwrap();
                let xp = host.hub.guests[0].xp_total;
                let delay = host.play.get(&1).map_or(0, |p| p.xp_delay);
                out = (state, xp, delay, saved);
            });
        });
        out
    }

    fn run_pickup(
        initial_value: i32,
        initial_count: u32,
        initial_xp: i32,
        hooks: impl Hooks + 'static,
    ) -> (Option<(i32, u32)>, i32, u32, Option<Tag>) {
        run_pickup_mods(
            initial_value,
            initial_count,
            initial_xp,
            vec![ModEntry::new("pickup-xp", Box::new(hooks))],
        )
    }

    /// `PlayerXpEvent.PickupXp` through the shared `listened` probe: a pickup goes the
    /// same with no listener and with one that changes nothing, and a cancel keeps the orb.
    #[test]
    #[ignore = "daily"]
    fn pickup_xp_is_the_same_with_and_without_a_listener() {
        let none = take_orb();
        assert_eq!(none, (false, 7));
        let mods = vec![ModEntry::new("listen", Box::new(Listen(false)))].into();
        mod_platform::testing::with_mods(mods, || assert_eq!(take_orb(), none));
        let mods = vec![ModEntry::new("cancel", Box::new(Listen(true)))].into();
        mod_platform::testing::with_mods(mods, || assert_eq!(take_orb(), (true, 0)));
    }

    /// `ExperienceOrbRef::set_value` keeps vanilla `ExperienceOrb.DATA_VALUE` signed `int`
    /// semantics without clamping `<= 0` to 1; surviving orbs persist `Value` as a signed short.
    #[test]
    #[ignore = "daily"]
    fn pickup_xp_value_edits_keep_signed_vanilla_semantics_independently_of_count() {
        let (orb, xp, delay, saved) = run_pickup(
            7,
            2,
            10,
            Edit {
                value: Some(0),
                count: None,
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((0, 1)), 10, 2));
        let back = orbs_from_nbt(&saved.unwrap());
        assert_eq!((back[0].value, back[0].count), (0, 1));

        let (orb, xp, delay, saved) = run_pickup(
            7,
            2,
            10,
            Edit {
                value: Some(-5),
                count: None,
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((-5, 1)), 10, 2));
        let back = orbs_from_nbt(&saved.unwrap());
        assert_eq!((back[0].value, back[0].count), (-5, 1));

        let (orb, xp, delay, saved) = run_pickup(
            7,
            2,
            0,
            Edit {
                value: Some(40_000),
                count: None,
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((40_000, 1)), 40_000, 2));
        let tag = saved.unwrap();
        assert_eq!(tag.get("Value"), Some(&Tag::Short(40_000_i32 as i16)));
        assert_eq!(tag.get("Count"), Some(&Tag::Int(1)));
    }

    /// `ExperienceOrbRef::set_count` enforces vanilla `ExtraCodecs.POSITIVE_INT` (`1..=i32::MAX`)
    /// without mutating the orb on `0` or `> i32::MAX`; `1` takes one pickup and discards,
    /// `> 1` takes one pickup and leaves `count - 1`, and `EntityRef::kill` removes the orb
    /// without awarding XP or setting `takeXpDelay`.
    #[test]
    #[ignore = "daily"]
    fn pickup_xp_count_edits_enforce_positive_int_and_kill_discards_without_delay() {
        let (orb, xp, delay, _) = run_pickup(
            7,
            3,
            0,
            Edit {
                value: None,
                count: Some(0),
                expect_count_err: Some(ExperienceOrbWriteError::InvalidCount),
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((7, 2)), 7, 2));

        let (orb, xp, delay, _) = run_pickup(
            7,
            3,
            0,
            Edit {
                value: None,
                count: Some(i32::MAX as u32 + 1),
                expect_count_err: Some(ExperienceOrbWriteError::InvalidCount),
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((7, 2)), 7, 2));

        let (orb, xp, delay, _) = run_pickup(
            7,
            3,
            0,
            Edit {
                value: None,
                count: Some(1),
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (None, 7, 2));

        let (orb, xp, delay, saved) = run_pickup(
            7,
            1,
            0,
            Edit {
                value: None,
                count: Some(4),
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay), (Some((7, 3)), 7, 2));
        let back = orbs_from_nbt(&saved.unwrap());
        assert_eq!((back[0].value, back[0].count), (7, 3));

        let (orb, xp, delay, saved) = run_pickup(
            7,
            3,
            0,
            Edit {
                value: None,
                count: None,
                expect_count_err: None,
                kill: true,
                cancel: false,
                calls: Arc::new(AtomicUsize::new(0)),
            },
        );
        assert_eq!((orb, xp, delay, saved), (None, 0, 0, None));
    }

    /// A veto skips the pickup continuation (`takeXpDelay` stays 0, no XP or count decrement)
    /// while leaving live orb writes in place; `takeXpDelay > 0` skips firing `pickup_xp`
    /// until it reaches 0; and later listeners immediately observe earlier live orb edits.
    #[test]
    #[ignore = "daily"]
    fn pickup_xp_cancel_cooldown_and_multi_listener_live_orb_edits() {
        let cancel_calls = Arc::new(AtomicUsize::new(0));
        let (orb, xp, delay, _) = run_pickup(
            7,
            2,
            0,
            Edit {
                value: Some(99),
                count: Some(5),
                expect_count_err: None,
                kill: false,
                cancel: true,
                calls: Arc::clone(&cancel_calls),
            },
        );
        assert_eq!(cancel_calls.load(Ordering::Relaxed), 1);
        assert_eq!((orb, xp, delay), (Some((99, 5)), 0, 0));

        let calls = Arc::new(AtomicUsize::new(0));
        let mods = vec![ModEntry::new(
            "pickup-cooldown",
            Box::new(Edit {
                value: None,
                count: None,
                expect_count_err: None,
                kill: false,
                cancel: false,
                calls: Arc::clone(&calls),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                open_to_lan(game);
                let mut g = guest(1);
                g.loaded = true;
                g.position = [0.5, 70.0, 0.5];
                game.server.lan.as_mut().unwrap().hub.guests.push(g);
                game.orbs.push(make_orb(7, 2));

                game.guest_orbs(0);
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].xp_total, 7);
                assert_eq!(game.server.lan.as_ref().unwrap().play[&1].xp_delay, 2);

                // Tick while delayed: takeXpDelay drops 2 -> 1 and pickup_xp does not fire.
                game.guest_orbs(0);
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].xp_total, 7);
                assert_eq!(game.server.lan.as_ref().unwrap().play[&1].xp_delay, 1);

                // Next tick: takeXpDelay drops 1 -> 0, pickup_xp fires and takes the last orb.
                game.guest_orbs(0);
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                assert_eq!(game.server.lan.as_ref().unwrap().hub.guests[0].xp_total, 14);
                assert_eq!(game.server.lan.as_ref().unwrap().play[&1].xp_delay, 2);
                assert!(game.orbs.is_empty());
            });
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let (orb, xp, delay, _) = run_pickup_mods(
            7,
            1,
            0,
            vec![
                ModEntry::new(
                    "first",
                    Box::new(ObserveAndEdit {
                        seen: Arc::clone(&seen),
                        write_value: Some(15),
                        write_count: Some(3),
                    }),
                ),
                ModEntry::new(
                    "second",
                    Box::new(ObserveAndEdit {
                        seen: Arc::clone(&seen),
                        write_value: Some(21),
                        write_count: None,
                    }),
                ),
            ],
        );
        assert_eq!(*seen.lock().unwrap(), [(7, 1), (15, 3)]);
        assert_eq!((orb, xp, delay), (Some((21, 2)), 21, 2));
    }
}

#[cfg(test)]
#[path = "tests/menu_still_valid_tests.rs"]
pub(in crate::game) mod menu_still_valid_tests;

#[cfg(test)]
#[path = "tests/player_entity_tick_tests.rs"]
pub(in crate::game) mod player_entity_tick_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "tests/stop_menu.rs"]
pub mod stop_menu;
