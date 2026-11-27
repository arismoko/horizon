use std::sync::Arc;
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::CRATE_MENU;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::CrateMod;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::Frame;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::Heard;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::HopperMod;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::HopperScreen;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::PACK;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::Seen;
use sunrise_client_gameplay::game::native_hooks::screen_position_tests::with_listener;
use sunrise_client_gameplay::game::native_hooks::{
    OpenNativeMenu, OpenNativeScreen, seed_native_menu,
};
use sunrise_gameplay_common::game::Screen;
use sunrise_level::containers::BlockEntity;
use winit::dpi::PhysicalPosition;

#[test]
#[ignore = "daily"]
fn a_hopper_layout_mod_menu_opens_its_screen_from_block_packets() {
    use sunrise_level::containers::{ModBlockEntity, ModBlockEntityFields};
    use sunrise_protocol::{
        buf::Reader, connection::Pipe, menu_extra, packets::play::Clientbound as C,
    };
    use sunrise_sim::lan::{Out, PlayerSender};
    use {
        sunrise_gameplay::game::native_guest_turn_tests::register_test_menu,
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };
    const MENU: &str = "hopper_fixture:menu";
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::screen_position_tests::a_hopper_layout_mod_menu_opens_its_screen_from_block_packets",
        "SUNRISE_HOPPER_LAYOUT_CHILD",
        &["block"],
    ) {
        return;
    }
    let heard = Heard::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "hopper_fixture",
        Box::new(HopperMod {
            block_menu: MENU,
            screen: HopperScreen::<5> {
                owner: "hopper_fixture",
                menu: MENU,
                heard: heard.clone(),
            },
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        let pos = (1, 64, 1);
        sunrise_level::containers::with_mod_menu_types(
            [(
                MENU.into(),
                "hopper_fixture".into(),
                5,
                0,
                vec![],
                vec![],
                None,
                None,
                true,
                None,
            )],
            || {
                let mut sent = None;
                sunrise_gameplay::game::native_entity_refs::tests::with_game_role(
                    root.prepared().to_owned(),
                    ExecutionRole::IntegratedServer,
                    |game| {
                        open_to_lan(game);
                        let (tx, rx) = std::sync::mpsc::channel();
                        let mut player = guest(1);
                        player.brand = "sunrise".into();
                        player.sender = PlayerSender::new(tx, Arc::default());
                        player.sender.menu_extra_rev = menu_extra::LIVE_DATA_REVISION;
                        game.server.lan.as_mut().unwrap().hub.guests.push(player);
                        register_test_menu(game, MENU);
                        rx.try_iter().for_each(drop);
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("hopper_fixture:machine")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        let containers = game.native_containers();
                        game.world.block_entities.set_mod_containers(containers);
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "hopper_fixture:machine".into(),
                                ModBlockEntityFields {
                                    items: vec![None; 5],
                                    ..Default::default()
                                },
                            ))),
                        );
                        assert_eq!(game.mod_menu_at(pos), Some(MENU));
                        assert_eq!(
                            game.lan_open_container(1, pos),
                            sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                        );
                        let hub = &game.server.lan.as_ref().unwrap().hub;
                        let menu = hub.guest(1).unwrap().menu.as_ref().unwrap();
                        assert_eq!((&*menu.registry_id, menu.slot_count()), (MENU, 41));
                        let packets: Vec<_> = rx
                            .try_iter()
                            .flat_map(|out| match out {
                                Out::Packet(body) => vec![body],
                                Out::Packets(bodies) => bodies,
                                _ => vec![],
                            })
                            .filter_map(|body| {
                                let mut reader = Reader::new(&body);
                                let id = reader.varint().unwrap();
                                match C::decode(id, reader.rest()).unwrap() {
                                    C::CustomPayload(p) if p.channel == menu_extra::CHANNEL => {
                                        Some(C::CustomPayload(p))
                                    }
                                    C::OpenScreen(p) => Some(C::OpenScreen(p)),
                                    _ => None,
                                }
                            })
                            .collect();
                        sent = Some((packets, hub.sync_data().builtin.clone(), state));
                    },
                );
                let (packets, builtin, state) = sent.unwrap();
                sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
                    root.prepared().to_owned(),
                    ExecutionRole::Client,
                    |game| {
                        let (client, _server) = Pipe::pair();
                        game.connect_local(client, "Hopper layout").unwrap();
                        let remote = game.client.remote.as_mut().unwrap();
                        remote.builtin = Arc::new(builtin);
                        Arc::make_mut(&mut remote.registries).menu_extra_revision =
                            menu_extra::LIVE_DATA_REVISION;
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        assert_eq!(packets.len(), 2);
                        for packet in packets {
                            game.handle_packet(packet).unwrap();
                        }
                        assert_eq!(game.client.screen, Screen::Inventory);
                        let open = game
                            .client
                            .mod_screen
                            .0
                            .as_ref()
                            .expect("registered screen for the mod menu");
                        assert_eq!(open.menu.name, MENU);
                        assert_eq!(open.menu.block.as_deref(), Some("hopper_fixture:machine"));
                        assert_eq!(open.layout().slots.len(), 5);
                        for (slot, x) in [44.0, 62.0, 80.0, 98.0, 116.0].into_iter().enumerate() {
                            assert_eq!(game.screen_slot(MENU, slot), Some([x, 20.0]));
                        }
                        assert_eq!(*heard.lock().unwrap(), [("hopper_fixture", "create")]);
                    },
                );
            },
        );
    });
}

/// A mod block whose container opens a custom menu type: the client lays it
/// out for that opener, leaving slot 4 initially inactive (`None`).
#[test]
#[ignore = "daily"]
fn a_mod_blocks_custom_menu_gets_its_registered_screen() {
    // Content registries are process-global.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::screen_position_tests::a_mod_blocks_custom_menu_gets_its_registered_screen",
        "SUNRISE_SCREEN_POSITION_CRATE_CHILD",
        &["crate"],
    ) {
        return;
    }
    let seen = Seen::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(CrateMod(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        sunrise_level::containers::with_mod_menu_types(
            [(
                CRATE_MENU.into(),
                "test".into(),
                27,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                true,
                None,
            )],
            || {
                sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
                    root.prepared().to_owned(),
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    |game| {
                        seed_native_menu(game, 1, CRATE_MENU);
                        game.player = Some(
                            sunrise_sim::motion::PlayerMotion::new(
                                game.motion,
                                sunrise_core::geometry::Vec3::new(17.5, 64.0, -8.5),
                            )
                            .unwrap(),
                        );
                        let pos = (17, 63, -9);
                        game.world
                            .chunks
                            .insert((1, -1), Arc::new(sunrise_level::chunk::Chunk::empty(1, -1)));
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("test:crate")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        assert!(game.open_native_screen(pos, Some(pos), 27, None));
                        let open = game
                            .client
                            .mod_screen
                            .0
                            .as_ref()
                            .expect("the menu type's screen");
                        assert_eq!(open.menu.block.as_deref(), Some("test:crate"));
                        assert_eq!(open.layout().height, 166.0);
                        assert!(open.layout().slots[4].is_none());
                        game.container = Some(pos);
                        game.client.title.0 = None;
                        game.client.chat_ui.hide_gui = true;
                        game.client.settings.gui_scale = 2;
                        game.level.recipe_book.open = [false; 4];
                        game.world
                            .block_entities
                            .insert(pos, BlockEntity::Chest(vec![None; 27]));
                        game.set_screen(Screen::Inventory);
                        // No new motion: the odd-sized resize allows GUI scale 2.
                        let small = [638, 478];
                        let projection = game.pointer_projection(small);
                        let sample = projection
                            .window
                            .physical_sample(PhysicalPosition::new(321.75, 231.75));
                        game.set_window_cursor(sample, projection);
                        game.hud(small);
                        game.hud([641, 481]);

                        // A cursor supplied in GUI pixels directly still truncates toward zero.
                        game.client.window_cursor = None;
                        game.client.cursor = (-3.75, 129.75);
                        game.hud([641, 481]);
                        {
                            let seen = seen.lock().unwrap();
                            let block = Some("test:crate".to_owned());
                            assert_eq!(seen.draw, vec![(block, Some([17, 63, -9])); 3]);
                            assert_eq!(
                                seen.frames,
                                [
                                    Frame {
                                        pointer: [321, 231],
                                        size: [638, 478],
                                        origin: [231.0, 156.0],
                                    },
                                    Frame {
                                        pointer: [161, 116],
                                        size: [321, 241],
                                        origin: [72.0, 37.0],
                                    },
                                    Frame {
                                        pointer: [-3, 129],
                                        size: [321, 241],
                                        origin: [72.0, 37.0],
                                    },
                                ]
                            );
                            assert_eq!(
                                seen.tooltip_at,
                                [[90.0, 75.0], [89.0, 79.0], [-75.75, 92.75]]
                            );
                        }
                        assert_eq!(game.menu_opener_block(pos, CRATE_MENU), Some("test:crate"));
                    },
                );
            },
        );
    });
}

/// #1172: a mod menu type opened without a block (an item's use, an entity, a command) gets
/// its type's screen with omitted trailing slots starting inactive (`None`), while a vanilla
/// menu keeps its built-in screen.
#[test]
#[ignore = "daily"]
fn a_mod_menu_opened_without_a_block_gets_its_types_screen() {
    sunrise_level::containers::with_mod_menu_types(
        [(
            PACK.into(),
            "test".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        )],
        || {
            with_listener(|game, seen| {
                let key = (i32::MIN, 0, 0);
                seed_native_menu(game, 1, PACK);
                assert!(game.open_native_screen(key, None, 9, None));
                let open = game
                    .client
                    .mod_screen
                    .0
                    .as_ref()
                    .expect("the menu type's screen");
                assert_eq!((open.layout().height, &open.menu.block), (133.0, &None));
                assert_eq!(open.layout().slots, vec![None; 9]);
                game.container = Some(key);
                game.set_screen(Screen::Inventory);
                let mut batch = HudBatch::scratch([128, 128]);
                assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                assert_eq!(seen.lock().unwrap().draw, [(None, None)]);
                seed_native_menu(game, 2, "minecraft:generic_9x3");
                assert!(game.open_native_screen(key, None, 27, None));
                assert!(
                    game.client.mod_screen.0.is_none(),
                    "a vanilla chest keeps its screen"
                );
            });
        },
    );
}

/// Screen factories route directly by menu type (including cross-mod registration),
/// any mod may read/move/activate slots of the open screen, and a vanilla hopper keeps
/// vanilla's built-in screen.
#[test]
#[ignore = "daily"]
fn custom_menu_screens_route_by_menu_type_and_allow_cross_mod_slot_controls() {
    // Content registries are process-global.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::screen_position_tests::custom_menu_screens_route_by_menu_type_and_allow_cross_mod_slot_controls",
        "SUNRISE_SCREEN_POSITION_HOPPER_CHILD",
        &["hoppers"],
    ) {
        return;
    }
    let heard = Heard::default();
    let alpha_screen = HopperScreen::<0> {
        owner: "alpha",
        menu: "beta:hopper",
        heard: heard.clone(),
    };
    let beta_screen = HopperScreen::<4> {
        owner: "beta",
        menu: "alpha:hopper",
        heard: heard.clone(),
    };
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new(
            "alpha",
            Box::new(HopperMod {
                block_menu: "alpha:hopper",
                screen: alpha_screen,
            }),
        ),
        mod_api::ModEntry::new(
            "beta",
            Box::new(HopperMod {
                block_menu: "beta:hopper",
                screen: beta_screen,
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        sunrise_level::containers::with_mod_menu_types(
            [
                (
                    "alpha:hopper".into(),
                    "alpha".into(),
                    5,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                ),
                (
                    "beta:hopper".into(),
                    "beta".into(),
                    5,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                ),
            ],
            || {
                sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
                    root.prepared().to_owned(),
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    |game| {
                        use sunrise_protocol::packets::play::Serverbound;
                        let (mut peer, _writer) =
                            crate::game::remote::tests::connect_menu_test(game);
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        for (x, block, menu_name, hidden, screen_owner) in [
                            (1, "alpha:machine", "alpha:hopper", 4, "beta"),
                            (2, "beta:machine", "beta:hopper", 0, "alpha"),
                        ] {
                            let pos = (x, 64, 0);
                            let state = game.pack.assets.registry.default_state(block).unwrap();
                            game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                            heard.lock().unwrap().clear();
                            seed_native_menu(game, x, menu_name);
                            assert!(game.open_native_screen(pos, Some(pos), 5, None));
                            game.container = Some(pos);
                            game.set_screen(Screen::Inventory);
                            let open = game
                                .client
                                .mod_screen
                                .0
                                .as_ref()
                                .expect("the machine's screen");
                            let (menu, open_block, base) = (
                                open.menu.name.clone(),
                                open.menu.block.clone().unwrap(),
                                open.container_slot_base(),
                            );
                            assert_eq!(menu, menu_name);
                            assert_eq!(open_block, block);
                            let shown: Vec<_> = (0..5).filter(|&i| i != hidden).collect();
                            let ids: Vec<_> = shown.iter().map(|i| base + i).collect();
                            assert_eq!(game.native_screen_slots(), Some(ids));
                            let mut batch = HudBatch::scratch([512, 512]);
                            let (ox, oy) = game.inventory_origin(&batch);
                            game.client.cursor = (ox + 90.0, oy + 105.0);
                            assert!(game.draw_native_screen(&mut batch, ox, oy));
                            assert!(game.native_screen_press_at(&batch, false));
                            let click = (0..32).find_map(|_| {
                                let (id, body) = peer.recv().ok()?;
                                match Serverbound::decode(id, &body).unwrap() {
                                    Serverbound::ContainerButtonClick(click) => Some(click),
                                    _ => None,
                                }
                            });
                            let click = click.expect("the active hopper's button");
                            assert_eq!((click.container, click.button), (x, hidden as i32));
                            let mut lines = Vec::new();
                            game.native_screen_tooltip_lines(&batch, None, &mut lines);
                            assert_eq!(
                                lines,
                                [(sunrise_game_data::text::Text::literal(screen_owner), 0)]
                            );
                            let hooks = ["create", "draw", "click", "tooltip"];
                            let only_owner: Vec<_> =
                                hooks.iter().map(|&hook| (screen_owner, hook)).collect();
                            assert_eq!(*heard.lock().unwrap(), only_owner);
                            // Any mod may read, move, activate, and hide slots on the open screen.
                            for calling in ["alpha", "beta"] {
                                let _call = mod_api::content::crash::enter_native_mod(
                                    mod_api::content::crash::ModCall {
                                        id: calling,
                                        hook: "menu_screen_draw",
                                        part: None,
                                    },
                                );
                                assert!(game.screen_slot(&menu, shown[0]).is_some());
                                assert_eq!(
                                    game.set_screen_slot(&menu, shown[0], Some([1.0, 1.0])),
                                    Ok(())
                                );
                                assert_eq!(
                                    game.set_screen_slot(&menu, hidden, Some([20.0, 20.0])),
                                    Ok(())
                                );
                                assert_eq!(game.screen_slot(&menu, hidden), Some([20.0, 20.0]));
                                assert_eq!(game.set_screen_slot(&menu, hidden, None), Ok(()));
                                assert_eq!(game.screen_slot(&menu, hidden), None);
                            }
                        }
                        // A vanilla hopper asks neither mod.
                        let pos = (3, 64, 0);
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:hopper")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        heard.lock().unwrap().clear();
                        seed_native_menu(game, 3, "minecraft:hopper");
                        assert!(game.open_native_screen(pos, Some(pos), 5, None));
                        assert!(
                            game.client.mod_screen.0.is_none(),
                            "a vanilla hopper keeps its screen"
                        );
                        assert_eq!(game.menu_opener_block(pos, "minecraft:hopper"), None);
                        assert!(heard.lock().unwrap().is_empty());
                    },
                );
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn slotless_screen_draw_sees_full_width_updates_only_for_its_window() {
    use sunrise_protocol::{
        connection::Pipe,
        menu_extra::{DATA_CHANNEL, DataSlot, LIVE_DATA_REVISION},
        packets::{
            configuration::CustomPayload,
            play::{Clientbound as C, ContainerClose, OpenScreen},
        },
    };
    sunrise_level::containers::with_mod_menu_types(
        [(
            PACK.into(),
            "test".into(),
            0,
            2,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        )],
        || {
            with_listener(|game, seen| {
                let (pipe, _server) = Pipe::pair();
                game.connect_local(pipe, "Slotless").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                Arc::make_mut(&mut remote.registries).menu_extra_revision = LIVE_DATA_REVISION;
                Arc::make_mut(&mut remote.builtin).registries.insert(
                    "minecraft:menu".into(),
                    sunrise_protocol::registry::Ids::from_names(
                        ["minecraft:anvil", PACK].map(str::to_owned).to_vec(),
                    ),
                );
                let open =
                    |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, window, menu| {
                        game.handle_packet(C::OpenScreen(OpenScreen {
                            container: window,
                            menu,
                            title: sunrise_core::nbt::Tag::String("Test".into()),
                        }))
                        .unwrap();
                    };
                let receive = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                               window,
                               index,
                               value| {
                    game.handle_packet(C::CustomPayload(CustomPayload {
                        channel: DATA_CHANNEL.into(),
                        data: DataSlot {
                            window,
                            index,
                            value,
                        }
                        .encode(),
                    }))
                    .unwrap();
                };
                let draw = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let mut batch = HudBatch::scratch([200, 200]);
                    assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                };
                open(game, 1, 0); // Anvil
                assert!(game.station.is_some());
                assert!(!game.slots().is_empty());
                game.client.cursor = (25.0, 40.0);
                open(game, 2, 1); // No intervening close packet.
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.client.cursor, (25.0, 40.0));
                assert!(game.station.is_none());
                assert!(game.slots().is_empty());
                assert_eq!(game.slot_count(), 0);
                assert_eq!(crate::game::inventory_screen_slot(PACK, 0), None);
                assert_eq!(
                    game.slot_at(&HudBatch::scratch([200, 200]), 25.0, 40.0),
                    None
                );
                seen.lock().unwrap().cancel_opening = true;
                game.set_screen(Screen::Paused);
                assert_eq!(game.client.screen, Screen::Inventory);
                assert!(
                    game.native_screen_shown(),
                    "an unrelated veto keeps the menu"
                );
                seen.lock().unwrap().cancel_opening = false;
                receive(game, 2, 0, -80_000);
                receive(game, 2, 1, 3);
                draw(game);
                receive(game, 2, 2, 9);
                receive(game, 2, 0, i32::MAX);
                draw(game);
                let first = game.client.remote.as_ref().unwrap().menu.generation;
                // Even before the old state is dropped, another generation
                // of the same window cannot see it or write to it.
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((2, PACK.into())));
                assert!(game.shown_native_screen().is_none());
                assert!(!game.native_screen_data(2, 0, 8));
                open(game, 2, 1); // Same id, new opening.
                assert_ne!(game.client.remote.as_ref().unwrap().menu.generation, first);
                receive(game, 3, 0, 8);
                draw(game);
                receive(game, 2, 0, -17);
                receive(game, 2, 1, i32::MIN);
                draw(game);
                game.handle_packet(C::ContainerClose(ContainerClose { container: 2 }))
                    .unwrap();
                receive(game, 2, 1, 18);
                assert!(game.shown_native_screen().is_none());
                seen.lock().unwrap().cancel_opening = true;
                open(game, 2, 1);
                assert_eq!(game.client.screen, Screen::Playing);
                assert!(
                    game.client.mod_screen.0.is_none(),
                    "a veto drops the staged screen"
                );
                assert!(game.client.remote.as_ref().unwrap().menu.as_ref().is_none());
                assert_eq!(
                    seen.lock().unwrap().data,
                    [
                        vec![-80_000, 3],
                        vec![i32::MAX, 3],
                        vec![],
                        vec![-17, i32::MIN],
                    ]
                );
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn stock_menu_data_sync_round_trips_above_255_and_enforces_signed_short_boundary() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::screen_position_tests::stock_menu_data_sync_round_trips_above_255_and_enforces_signed_short_boundary",
        "SUNRISE_SCREEN_POSITION_DATA_SYNC_CHILD",
        &["sync"],
    ) {
        return;
    }
    use sunrise_level::{
        containers::{ModBlockEntity, ModBlockEntityFields},
        world::World,
    };
    use sunrise_protocol::{
        buf::Reader,
        packets::{
            configuration::ClientInformation,
            play::{Clientbound as C, ContainerSetData},
        },
    };
    use sunrise_sim::lan::{
        Joined, Lan, NetConfig, Out, PlayerSender,
        chunks::ChunkEncoder,
        data::SyncData,
        guest::Guest,
        hub::{Hub, HubConfig},
        menu::{Menu, Slots},
    };

    let seen = Seen::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(CrateMod(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
            root.prepared().to_owned(),
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            |game| {
                const MENU: &str = "minecraft:generic_9x3";
                let generation = seed_native_menu(game, 1, MENU);
                let pos = (17, 63, -9);
                game.world
                    .chunks
                    .insert((1, -1), Arc::new(sunrise_level::chunk::Chunk::empty(1, -1)));
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("test:crate")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                assert!(game.open_native_screen(pos, Some(pos), 27, None));
                assert!(
                    game.client.mod_screen.0.is_none(),
                    "the stock menu has no mod factory"
                );
                // Stock screens cannot be replaced by registration. Stage a draw
                // probe to inspect the data delivered by the stock packet handler.
                let draw_seen = seen.clone();
                game.client.mod_screen.0 = Some(OpenNativeScreen {
                    menu: OpenNativeMenu {
                        key: pos,
                        window: 1,
                        generation,
                        name: MENU.into(),
                        title: None,
                        block: None,
                        pos: None,
                        mod_type: None,
                        data: Vec::new(),
                    },
                    slot_moves: Default::default(),
                    layout_revision: 0,
                    shown: false,
                    detached: false,
                    screen: mod_api::__private::OwnedMenuScreen::new(
                        "test",
                        0,
                        mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                            width: 176.0,
                            height: 166.0,
                            own_background: true,
                            own_labels: true,
                            ..Default::default()
                        })
                        .with_draw(move |_, frame, _| {
                            draw_seen.lock().unwrap().data.push(frame.data.to_vec());
                        }),
                    ),
                });
                game.container = Some(pos);
                game.set_screen(Screen::Inventory);

                let config = NetConfig {
                    announce: false,
                    listen: false,
                    ..NetConfig::loopback(0, "StockDataSyncTest".into(), 2)
                };
                let sync = Arc::new(SyncData::empty());
                let lan = Lan::start_with(sync.clone(), config).unwrap();
                let encoder = ChunkEncoder::new(
                    [] as [(Arc<str>, bool); 0],
                    &sunrise_protocol::registry::Builtin::default(),
                    &[],
                    &[],
                    Default::default(),
                );
                let hub_config = HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 10,
                    simulation_distance: 10,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 20,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                };
                let mut hub = Hub::new(lan, sync, encoder, hub_config, game.loop_context.clone());
                let (tx, rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn: 1,
                        uuid: 1,
                        name: "Guest".into(),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: "sunrise".into(),
                        sender: PlayerSender::new(tx, Arc::default()),
                        latency: Arc::new(std::sync::atomic::AtomicI32::new(0)),
                        address: "127.0.0.1:25565".parse().unwrap(),
                        expected: false,
                    },
                    1,
                    [17.5, 64.0, -8.5],
                    8,
                );
                guest.placed = true;
                guest.dimension = "minecraft:overworld".into();
                guest.menu = Some(Menu {
                    window: 1,
                    kind: 2,
                    pos: Some(pos),
                    slots: Slots::Block(pos),
                    size: 27,
                    state: 0,
                    sent: Vec::new(),
                    drag: None,
                    sent_data: Vec::new(),
                    reported_unsent_data_len: None,
                    blockless_data: Vec::new(),

                    mod_type: None,
                    registry_id: MENU.into(),
                    craft: None,
                    links: Vec::new(),
                    opening_item: None,
                    asks_still_valid: false,
                    opener_block: None,
                    removed: false,
                    slot_changed_seen: None,
                });
                hub.guests.push(guest);

                let max_wire_index = usize::from(i16::MAX as u16);
                let mut data = vec![0i32; 65_537];
                data[0] = 11;
                data[300] = 22;
                data[max_wire_index] = 33;
                data[max_wire_index + 1] = 44;
                data[65_536] = 55;
                let mut server_world = World::new(game.res.facts().air, 0);
                server_world.block_entities.insert(
                    pos,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "test:crate".into(),
                        ModBlockEntityFields {
                            data,
                            ..Default::default()
                        },
                    ))),
                );

                hub.sync_containers(&server_world);
                let packets: Vec<C> = rx
                    .try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        _ => Vec::new(),
                    })
                    .filter_map(|body| {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        match C::decode(id, reader.rest()).unwrap() {
                            packet @ C::ContainerSetData(_) => Some(packet),
                            _ => None,
                        }
                    })
                    .collect();
                assert_eq!(packets.len(), max_wire_index + 1);
                for packet in packets {
                    game.handle_packet(packet).unwrap();
                }

                // Signed-short negative wire IDs must be rejected instead of reinterpreted as u16.
                for negative in [-1i16, i16::MIN] {
                    game.handle_packet(C::ContainerSetData(ContainerSetData {
                        container: 1,
                        id: negative,
                        value: 99,
                    }))
                    .unwrap();
                }

                let mut batch = HudBatch::scratch([200, 200]);
                assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                let frames = &seen.lock().unwrap().data;
                assert_eq!(frames.len(), 1);
                let frame = &frames[0];
                assert_eq!(frame.len(), max_wire_index + 1);
                assert_eq!(frame[0], 11);
                assert_eq!(frame[1], 0);
                assert_eq!(frame[300], 22);
                assert_eq!(frame[max_wire_index], 33);
            },
        );
    });
}

/// A mod screen's player slots take the same hides and moves as its container slots, in
/// menu order after them (the main inventory's rows, then the hotbar), and a screen opened
/// again starts from the declared places.
#[test]
fn player_slots_follow_the_container_slots_in_menu_order() {
    use mod_api::ScreenSlotRefusal::{BadPosition, NoSuchSlot};
    fn open(player_inventory: bool) -> OpenNativeScreen {
        let kind: &'static sunrise_level::containers::ModMenuType =
            Box::leak(Box::new(sunrise_level::containers::ModMenuType {
                id: "test:player_slots",
                owner: "test",
                slots: 2,
                player_inventory,
                data_slots: 0,
                owned_slots: None,
                results: vec![],
                inputs: Vec::new(),
                links: None,
                crafting: None,
            }));
        OpenNativeScreen {
            menu: OpenNativeMenu {
                key: (0, 0, 0),
                window: 1,
                generation: 1,
                name: "test:player_slots".into(),
                title: None,
                block: None,
                pos: None,
                mod_type: Some(kind),
                data: Vec::new(),
            },
            slot_moves: Default::default(),
            layout_revision: 0,
            shown: true,
            detached: false,
            screen: mod_api::__private::OwnedMenuScreen::new(
                "test",
                0,
                mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![
                        Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            ..Default::default()
                        }),
                        None,
                    ],
                    inventory_x: 8.0,
                    inventory_y: 84.0,
                    ..Default::default()
                }),
            ),
        }
    }
    fn latest(s: &OpenNativeScreen, slot: usize) -> Option<[f32; 2]> {
        s.slot_moves.latest(slot, s.declared_slot(slot))
    }
    fn set(
        s: &OpenNativeScreen,
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        s.slot_moves.set(slot, s.declared_slot(slot), pos)
    }
    // Two container slots, so the player's slots are menu slots 2..38: the main
    // inventory's 27 (inventory slots 9..36), then the hotbar's 9 (inventory slots 0..9).
    let s = open(true);
    let base = s.container_slot_base();
    assert_eq!(latest(&s, 2), Some([8.0, 84.0]));
    assert_eq!(latest(&s, 2 + 8), Some([152.0, 84.0]));
    assert_eq!(latest(&s, 2 + 9), Some([8.0, 102.0]));
    assert_eq!(latest(&s, 2 + 26), Some([152.0, 120.0]));
    assert_eq!(latest(&s, 2 + 27), Some([8.0, 142.0]));
    assert_eq!(latest(&s, 2 + 35), Some([152.0, 142.0]));
    assert_eq!(latest(&s, 38), None);
    assert_eq!(set(&s, 38, None), Err(NoSuchSlot));
    assert_eq!(set(&s, usize::MAX, Some([0.0, 0.0])), Err(NoSuchSlot));
    assert_eq!(s.player_menu_slot(9), Some(2));
    assert_eq!(s.player_menu_slot(35), Some(28));
    assert_eq!(s.player_menu_slot(0), Some(29));
    assert_eq!(s.player_menu_slot(8), Some(37));
    assert_eq!(s.player_menu_slot(36), None);
    let all: Vec<usize> = (0..36).chain([base]).collect();
    assert_eq!(s.visible_slots().collect::<Vec<_>>(), all);

    // A hide, pending until the build commits it, counts once and only once.
    assert_eq!(set(&s, 2, None), Ok(()));
    assert_eq!(s.slot_move_count(), 1);
    assert_eq!(latest(&s, 2), None);
    assert_eq!(s.visible_slots().collect::<Vec<_>>(), all);
    assert_eq!(set(&s, 2, None), Ok(()));
    assert_eq!(s.slot_move_count(), 1);
    // The hotbar's first slot, then a move of the main inventory's second.
    assert_eq!(set(&s, 29, None), Ok(()));
    assert_eq!(set(&s, 3, Some([30.0, 40.0])), Ok(()));
    assert_eq!(s.slot_move_count(), 3);
    assert_eq!(set(&s, 3, Some([30.0, 40.0])), Ok(()));
    assert_eq!(set(&s, 4, Some([26.0, 84.0])), Ok(()));
    assert_eq!(s.slot_move_count(), 4);
    // Refusals change nothing.
    assert_eq!(set(&s, 5, Some([f32::NAN, 0.0])), Err(BadPosition));
    assert_eq!(set(&s, 5, Some([0.0, f32::INFINITY])), Err(BadPosition));
    assert_eq!(s.slot_move_count(), 4);
    s.commit_slot_moves();
    let shown: Vec<usize> = (1..36).filter(|&slot| slot != 9).chain([base]).collect();
    assert_eq!(s.visible_slots().collect::<Vec<_>>(), shown);
    assert_eq!(s.player_slot_position(9), None);
    assert_eq!(s.player_slot_position(0), None);
    assert_eq!(s.player_slot_position(10), Some([30.0, 40.0]));
    assert_eq!(s.player_slot_position(11), Some([26.0, 84.0]));
    assert_eq!(s.player_slot_position(12), Some([8.0 + 3.0 * 18.0, 84.0]));
    assert_eq!(s.player_slot_position(1), Some([26.0, 142.0]));
    // The container slots keep their own places.
    assert_eq!(latest(&s, 0), Some([8.0, 18.0]));
    assert_eq!(latest(&s, 1), None);

    // Shown again where the mod puts it, or where it was.
    assert_eq!(set(&s, 2, Some([8.0, 84.0])), Ok(()));
    assert_eq!(set(&s, 29, Some([100.0, 100.0])), Ok(()));
    assert_eq!(s.slot_move_count(), 6);
    s.commit_slot_moves();
    assert_eq!(s.player_slot_position(9), Some([8.0, 84.0]));
    assert_eq!(s.player_slot_position(0), Some([100.0, 100.0]));
    let mut again: Vec<usize> = (0..36).collect();
    again.push(base);
    assert_eq!(s.visible_slots().collect::<Vec<_>>(), again);

    // Closing and opening again is a new screen: every slot is back where it was declared.
    let reopened = open(true);
    assert_eq!(reopened.slot_move_count(), 0);
    assert_eq!(reopened.visible_slots().collect::<Vec<_>>(), all);
    assert_eq!(reopened.player_slot_position(10), Some([26.0, 84.0]));
    assert_eq!(reopened.player_slot_position(0), Some([8.0, 142.0]));

    // A menu with no player inventory has no player slots to address.
    let bare = open(false);
    assert_eq!(latest(&bare, 2), None);
    assert_eq!(set(&bare, 2, None), Err(NoSuchSlot));
    assert_eq!(set(&bare, 2, Some([f32::NAN, 0.0])), Err(NoSuchSlot));
    assert_eq!(bare.slot_move_count(), 0);
    assert_eq!(bare.visible_slots().collect::<Vec<_>>(), [base]);
    assert_eq!(set(&bare, 1, Some([26.0, 18.0])), Ok(()));
}
