use std::sync::Arc;
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::BookMod;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::BoundBookMod;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::BoundLayoutEdit;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::KEY;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::MENU;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::REOPENED_MENU;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::ReopenedMod;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::STONE;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::ScreenTrace;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::geometry;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::receive;
use sunrise_client_gameplay::game::native_hooks::book_screen::lifecycle_tests_shared::with_bound_menu_type;
use sunrise_client_gameplay::game::native_hooks::{
    OpenNativeMenu, OpenNativeScreen, seed_native_menu,
};
use sunrise_gameplay_common::game::Screen;
use sunrise_sim::recipe_book::{BookFlags, placement_sync::ResourceId, settings_sync::Message};

/// Slot moves and hides keep the catalogue and nonce. An in-place open of
/// the same numeric window discards both, as well as the old layout.
#[test]
#[ignore = "daily"]
fn a_bound_books_catalogue_survives_slot_moves_and_hides() {
    use sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE;
    use sunrise_protocol::packets::play::{Clientbound, OpenScreen};
    use sunrise_sim::recipe_book::placement_sync::CatalogIdentity;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bound",
        Box::new(BoundBookMod::default()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_bound_menu_type(|| {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Bound").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                remote.custom_books = true;
                remote.joined = true;
                Arc::make_mut(&mut remote.builtin).registries.insert(
                    "minecraft:menu".into(),
                    sunrise_protocol::registry::Ids::from_names(vec!["bound:menu".into()]),
                );
                game.client.settings.gui_scale = 1;
                game.client.recipe_ui.0.test_viewport = Some([1280, 720]);
                game.client.title.0 = None;
                let open = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.handle_packet(Clientbound::OpenScreen(OpenScreen {
                        container: 1,
                        menu: 0,
                        title: sunrise_core::nbt::Tag::String("Bound".into()),
                    }))
                    .unwrap();
                };
                game.native_net.placement.client.hello(6);
                assert!(
                    game.native_net
                        .placement
                        .client
                        .ready(6, sunrise_sim::recipe_book::placement_sync::schema())
                );
                open(game);
                let first = game.client.remote.as_ref().unwrap().menu.generation;
                let window = game.native_net.placement.client.window().unwrap().clone();
                assert_eq!(window.window, 1);

                // The server's catalogue answers the client's first request.
                use sunrise_sim::recipe_book::{
                    menu_layout::MenuLayoutMetadata,
                    placement_sync::{DisplayId, Frame, encode_catalogue},
                };
                let identity = CatalogIdentity {
                    epoch: 6,
                    number: 1,
                    window: window.window,
                    nonce: window.nonce,
                    instance: 1,
                    generation: 1,
                    menu: window.menu.clone(),
                    book: window.book.clone(),
                };
                let (_, _, layout) = mod_api::RecipeBookLayout::registered("bound:menu").unwrap();
                let bytes = encode_catalogue(&[]).unwrap();
                game.remote_mod_book_placement(Frame::Start {
                    identity: identity.clone(),
                    layout,
                    storage: MenuLayoutMetadata {
                        slots: 3,
                        crafting: None,
                    },
                    total: bytes.len() as u32,
                });
                game.remote_mod_book_placement(Frame::Part {
                    epoch: 6,
                    number: 1,
                    offset: 0,
                    bytes,
                });
                let kept = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.native_net.placement.client.catalog() == Some(&identity)
                        && game.native_net.placement.client.window() == Some(&window)
                        && game.native_book_placement_catalog_matches(&identity)
                        && game.native_book_placement_window_matches(&window)
                };
                assert!(kept(game));

                // The declaring mod keeps its slot controls for the active opening.
                {
                    let _call = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "bound",
                            hook: "render_gui_post",
                            part: None,
                        },
                    );
                    assert_eq!(game.screen_slot("bound:menu", 0), Some([8.0, 18.0]));
                    assert_eq!(
                        game.set_screen_slot("bound:menu", 0, Some([30.0, 40.0])),
                        Ok(())
                    );
                    assert_eq!(game.set_screen_slot("bound:menu", 1, None), Ok(()));
                }
                game.build_screen();
                game.ensure_mod_book_catalog();
                assert!(kept(game));
                assert_eq!(game.native_screen_slot(0), Some((30.0, 40.0)));
                assert_eq!(game.native_screen_slot(1), None);
                let batch = sunrise_client::hud::HudBatch::new(
                    [1280, 720],
                    game.client.settings.gui_scale_inputs(),
                );
                assert_eq!(game.slot_at(&batch, 582.0, 317.0), Some(MOD_MENU_SLOT_BASE));
                assert_ne!(
                    game.slot_at(&batch, 632.0, 295.0),
                    Some(MOD_MENU_SLOT_BASE + 1)
                );

                // Placements do not wait on a server reply; only a current server ghost
                // installs ghost slots, and a new recipe click or menu replacement clears it.
                let recipe = ResourceId::try_from("bound:recipe".to_owned()).unwrap();
                assert!(matches!(
                    game.native_net.placement.client.place(
                        DisplayId {
                            recipe: recipe.clone(),
                            source: 0,
                        },
                        false,
                    ),
                    Some(Frame::Place {
                        catalog,
                        use_max: false,
                        ..
                    }) if catalog == identity
                ));
                assert!(matches!(
                    game.native_net.placement.client.place(
                        DisplayId {
                            recipe: recipe.clone(),
                            source: 0,
                        },
                        true,
                    ),
                    Some(Frame::Place {
                        catalog,
                        use_max: true,
                        ..
                    }) if catalog == identity
                ));
                let ghost_display = mod_api::RecipeBookDisplay::Shapeless {
                    ingredients: vec![mod_api::RecipeSlotDisplay::Item("minecraft:stick".into())],
                    result: mod_api::RecipeSlotDisplay::Item("minecraft:stone".into()),
                    station: mod_api::RecipeSlotDisplay::Item("minecraft:crafting_table".into()),
                };
                game.remote_mod_book_placement(Frame::Ghost {
                    catalog: identity.clone(),
                    recipe: recipe.clone(),
                    display: ghost_display.clone(),
                });
                // Slot 0 is visible; slot 1 was hidden above and receives no ghost.
                assert_eq!(
                    game.ghost_slots(),
                    vec![(MOD_MENU_SLOT_BASE, "minecraft:stick".to_owned(), false)]
                );
                assert!(kept(game));
                game.request_mod_book_recipe("bound:recipe".to_owned(), 0);
                assert!(game.ghost_slots().is_empty());
                assert!(kept(game));

                game.remote_mod_book_placement(Frame::Ghost {
                    catalog: identity.clone(),
                    recipe: recipe.clone(),
                    display: ghost_display.clone(),
                });
                assert_eq!(
                    game.ghost_slots(),
                    vec![(MOD_MENU_SLOT_BASE, "minecraft:stick".to_owned(), false)]
                );

                // The server replaces the menu in place, with no close packet.
                open(game);
                assert_eq!(game.client.screen, Screen::Inventory);
                assert!(game.ghost_slots().is_empty());
                assert_ne!(game.client.remote.as_ref().unwrap().menu.generation, first);
                let replacement = game.native_net.placement.client.window().unwrap();
                assert_eq!(replacement.window, 1);
                assert_ne!(replacement.nonce, window.nonce);
                assert!(!game.native_book_placement_window_matches(&window));
                assert!(!game.native_book_placement_catalog_matches(&identity));
                assert_eq!(game.native_net.placement.client.catalog(), None);
                // A late ghost for the replaced opening cannot populate the new opening.
                game.remote_mod_book_placement(Frame::Ghost {
                    catalog: identity.clone(),
                    recipe,
                    display: ghost_display,
                });
                assert!(game.ghost_slots().is_empty());
                assert_eq!(game.native_screen_slot(0), Some((8.0, 18.0)));
                assert_eq!(game.native_screen_slot(1), Some((80.0, 18.0)));
                assert!(
                    geometry(game)
                        .1
                        .contains(&(MOD_MENU_SLOT_BASE, [559.0, 294.0, 18.0, 18.0]))
                );
                assert_eq!(
                    game.slot_at(&batch, 632.0, 295.0),
                    Some(MOD_MENU_SLOT_BASE + 1)
                );
            });
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_recipe_book_screen_refuses_bad_layouts_at_open() {
    let book_mod = BoundBookMod::default();
    let edit = book_mod.edit.clone();
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("bound", Box::new(book_mod)),
        mod_api::ModEntry::new("geometry", Box::new(BookMod)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_bound_menu_type(|| {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
                seed_native_menu(game, 1, "bound:menu");
                let key = (i32::MIN, 0, 0);
                let cases: [(&str, BoundLayoutEdit); 5] = [
                    ("missing book", |layout| layout.recipe_book = None),
                    ("wrong book", |layout| {
                        layout.recipe_book = Some(KEY.get().unwrap().clone());
                    }),
                    ("nonpositive width", |layout| layout.width = 0.0),
                    ("nonfinite slot", |layout| {
                        layout.slots[0].as_mut().unwrap().position[0] = f32::NAN;
                    }),
                    ("too many slots", |layout| {
                        layout.slots.resize(4, None);
                    }),
                ];
                for (case, adjust) in cases {
                    *edit.lock().unwrap() = Some(adjust);
                    assert!(
                        !game.open_native_screen(key, None, 3, None),
                        "accepted {case}"
                    );
                    assert!(game.client.mod_screen.0.is_none(), "staged {case}");
                }
                // Initially inactive slots (`None`) and unknown `no_item_icon` sprites open cleanly.
                for (case, adjust) in [
                    (
                        "inactive slot",
                        (|layout| layout.slots[1] = None) as BoundLayoutEdit,
                    ),
                    ("unknown sprite", |layout| {
                        layout.slots[0].as_mut().unwrap().no_item_icon =
                            Some("bound:missing_sprite".into());
                    }),
                ] {
                    *edit.lock().unwrap() = Some(adjust);
                    assert!(
                        game.open_native_screen(key, None, 3, None),
                        "refused {case}"
                    );
                    assert!(game.client.mod_screen.0.is_some(), "did not stage {case}");
                }
                *edit.lock().unwrap() = None;
                assert!(game.open_native_screen(key, None, 3, None));
            });
        });
    });
}

/// A hidden slot keeps its place this frame, then leaves drawing, listing and hit-testing from
/// the next build, and comes back where the mod puts it. Each refusal is named and changes
/// nothing; a slot hidden when the screen opened is never shown.
#[test]
#[ignore = "daily"]
fn a_hidden_slot_leaves_the_screen_and_comes_back_where_put() {
    use mod_api::ScreenSlotRefusal::*;
    use mod_api::prelude::ScreensClientHost;
    let mods: mod_api::Mods = Vec::<mod_api::ModEntry>::new().into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let generation = seed_native_menu(game, 1, MENU);
            game.client.settings.gui_scale = 1;
            game.client.recipe_ui.0.test_viewport = Some([1280, 720]);
            game.client.screen = Screen::Inventory;
            game.client.title.0 = None;
            game.container = Some((i32::MIN, 0, 0));
            game.client.mod_screen.0 = Some(OpenNativeScreen {
                menu: OpenNativeMenu {
                    key: (i32::MIN, 0, 0),
                    window: 1,
                    generation,
                    name: MENU.into(),
                    title: None,
                    block: Some(STONE.into()),
                    pos: None,
                    mod_type: None,
                    data: Vec::new(),
                },
                slot_moves: Default::default(),
                layout_revision: 0,
                shown: true,
                detached: false,
                screen: mod_api::__private::OwnedMenuScreen::new(
                    "geometry",
                    0,
                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![
                            Some(mod_api::ScreenSlot {
                                position: [8.0, 18.0],
                                ..Default::default()
                            }),
                            Some(mod_api::ScreenSlot {
                                position: [26.0, 18.0],
                                ..Default::default()
                            }),
                            None,
                        ],
                        inventory_y: 84.0,
                        inventory_x: 8.0,
                        ..Default::default()
                    }),
                ),
            });
            game.build_screen();
            let batch = sunrise_client::hud::HudBatch::new(
                [1280, 720],
                game.client.settings.gui_scale_inputs(),
            );
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));

            // Hidden by a render hook: still there this frame, gone from the next build.
            assert_eq!(game.set_screen_slot(MENU, 0, None), Ok(()));
            assert_eq!(ScreensClientHost::screen_slot(game, MENU, 0), None);
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), None);
            assert!(!geometry(game).1.iter().any(|(id, _)| *id == 300));
            assert_eq!(game.native_screen_slots(), Some(vec![301]));
            // The 36 player slots and the remaining visible container slot.
            assert_eq!(game.slot_count(), 37);
            assert_eq!(game.native_screen_slot(0), None);

            // Shown again where the mod puts it.
            assert_eq!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])), Ok(()));
            assert_eq!(game.screen_slot(MENU, 0), Some([48.0, 60.0]));
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), Some(300));
            assert_eq!(game.native_screen_slots(), Some(vec![300, 301]));
            assert_eq!(game.screen_slot(MENU, 1), Some([26.0, 18.0]));

            // A slot declared `None` at open starts inactive and can be activated and deactivated.
            assert_eq!(game.screen_slot(MENU, 2), None);
            assert_eq!(game.set_screen_slot(MENU, 2, None), Ok(()));
            assert_eq!(game.set_screen_slot(MENU, 2, Some([66.0, 18.0])), Ok(()));
            assert_eq!(game.screen_slot(MENU, 2), Some([66.0, 18.0]));
            game.build_screen();
            assert_eq!(game.native_screen_slot(2), Some((66.0, 18.0)));
            assert_eq!(game.native_screen_slots(), Some(vec![300, 301, 302]));
            assert_eq!(game.set_screen_slot(MENU, 2, None), Ok(()));
            game.build_screen();
            assert_eq!(game.native_screen_slot(2), None);
            assert_eq!(game.native_screen_slots(), Some(vec![300, 301]));

            // Refusals, each by name, and nothing changes.
            let before = geometry(game);
            // Three container slots and the 36 player slots: 39 is the first past the menu.
            assert_eq!(game.set_screen_slot(MENU, 39, None), Err(NoSuchSlot));
            assert_eq!(
                game.set_screen_slot(MENU, 0, Some([f32::NAN, 0.0])),
                Err(BadPosition)
            );
            assert_eq!(game.set_screen_slot("other:menu", 0, None), Err(NoScreen));
            assert_eq!(game.screen_slot("other:menu", 0), None);
            game.build_screen();
            assert_eq!(geometry(game), before);

            // With the screen closed there is no screen to change.
            game.client.screen = Screen::Playing;
            assert_eq!(game.set_screen_slot(MENU, 0, None), Err(NoScreen));
            assert_eq!(game.screen_slot(MENU, 0), None);
        });
    });
}

/// The player's slots take hides and moves like the container slots, in menu order after them
/// (the main inventory's rows, then the hotbar): a hidden one is not hovered, listed, drawn or
/// reached by the pointer, so a drag does not pick it up and a number key over it finds no slot,
/// and a screen opened again has every slot back. What the server does with a shift-click is not
/// decided here: nothing on the client filters it.
#[test]
#[ignore = "daily"]
fn hidden_player_slots_are_no_target_and_come_back_on_reopen() {
    use mod_api::prelude::ScreensClientHost;
    let mods: mod_api::Mods = Vec::<mod_api::ModEntry>::new().into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let generation = seed_native_menu(game, 1, MENU);
            game.client.settings.gui_scale = 1;
            game.client.recipe_ui.0.test_viewport = Some([1280, 720]);
            game.client.screen = Screen::Inventory;
            game.client.title.0 = None;
            game.container = Some((i32::MIN, 0, 0));
            let open = || OpenNativeScreen {
                menu: OpenNativeMenu {
                    key: (i32::MIN, 0, 0),
                    window: 1,
                    generation,
                    name: MENU.into(),
                    title: None,
                    block: Some(STONE.into()),
                    pos: None,
                    mod_type: None,
                    data: Vec::new(),
                },
                slot_moves: Default::default(),
                layout_revision: 0,
                shown: true,
                detached: false,
                screen: mod_api::__private::OwnedMenuScreen::new(
                    "geometry",
                    0,
                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![
                            Some(mod_api::ScreenSlot {
                                position: [8.0, 18.0],
                                ..Default::default()
                            }),
                            Some(mod_api::ScreenSlot {
                                position: [26.0, 18.0],
                                ..Default::default()
                            }),
                        ],
                        inventory_y: 84.0,
                        inventory_x: 8.0,
                        ..Default::default()
                    }),
                ),
            };
            game.client.mod_screen.0 = Some(open());
            game.build_screen();
            let batch = sunrise_client::hud::HudBatch::new(
                [1280, 720],
                game.client.settings.gui_scale_inputs(),
            );
            // Origin (552, 277): the main inventory's first slot (inventory slot 9) is at (8, 84)
            // and the hotbar's first (inventory slot 0) at (8, 142).
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(9));
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), Some(0));
            assert_eq!(game.slot_count(), 38);

            // Two container slots, so menu slot 2 is the main inventory's first and 29 the hotbar's.
            assert_eq!(game.screen_slot(MENU, 2), Some([8.0, 84.0]));
            assert_eq!(game.screen_slot(MENU, 29), Some([8.0, 142.0]));
            assert_eq!(game.set_screen_slot(MENU, 2, None), Ok(()));
            assert_eq!(game.set_screen_slot(MENU, 29, None), Ok(()));
            assert_eq!(ScreensClientHost::screen_slot(game, MENU, 2), None);
            // Still there this frame, gone from the next build.
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(9));
            game.build_screen();
            // Hover, and with it the hit test of a click, of a drag across it (which adds the slot
            // under the pointer) and of a number key over it (which swaps the slot under it).
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), None);
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), None);
            assert_eq!(game.slot_at(&batch, 579.0, 362.0), Some(10));
            assert_eq!(game.slot_at(&batch, 579.0, 420.0), Some(1));
            assert!(!geometry(game).1.iter().any(|(id, _)| *id == 9 || *id == 0));
            assert!(!game.slots().contains(&9) && !game.slots().contains(&0));
            assert_eq!(game.slot_count(), 36);
            let pad: Vec<[f32; 4]> = game
                .pad_slot_targets(&batch)
                .iter()
                .map(|t| t.rect)
                .collect();
            assert!(!pad.contains(&[560.0, 361.0, 16.0, 16.0]));
            assert!(!pad.contains(&[560.0, 419.0, 16.0, 16.0]));
            // The container slots are untouched.
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));

            // The hotbar's first slot shown elsewhere, the main inventory's where it was.
            assert_eq!(game.set_screen_slot(MENU, 29, Some([100.0, 20.0])), Ok(()));
            assert_eq!(game.set_screen_slot(MENU, 2, Some([8.0, 84.0])), Ok(()));
            game.build_screen();
            assert_eq!(game.slot_position(0), (100.0, 20.0));
            assert_eq!(game.slot_at(&batch, 653.0, 298.0), Some(0));
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), None);
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(9));

            // Past the menu's slots there is nothing to move.
            assert_eq!(
                game.set_screen_slot(MENU, 38, None),
                Err(mod_api::ScreenSlotRefusal::NoSuchSlot)
            );

            // Hidden again, then the screen is closed and opened again: every slot is back.
            assert_eq!(game.set_screen_slot(MENU, 2, None), Ok(()));
            assert_eq!(game.set_screen_slot(MENU, 29, None), Ok(()));
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), None);
            game.client.mod_screen.0 = Some(open());
            game.build_screen();
            assert_eq!(game.slot_count(), 38);
            assert_eq!(game.slot_position(0), (8.0, 142.0));
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(9));
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), Some(0));
            assert_eq!(game.screen_slot(MENU, 2), Some([8.0, 84.0]));
            assert_eq!(game.screen_slot(MENU, 29), Some([8.0, 142.0]));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_missing_recipe_book_factory_keeps_the_current_window() {
    use sunrise_protocol::packets::play::{Clientbound, OpenScreen};

    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bound",
        Box::new(BoundBookMod {
            no_factory: true,
            ..Default::default()
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_bound_menu_type(|| {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                const CHEST: &str = "minecraft:generic_9x3";
                assert!(mod_api::RecipeBookLayout::registered("bound:menu").is_some());
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Menu replacement").unwrap();
                Arc::make_mut(&mut game.client.remote.as_mut().unwrap().builtin)
                    .registries
                    .insert(
                        "minecraft:menu".into(),
                        sunrise_protocol::registry::Ids::from_names(vec![
                            CHEST.into(),
                            "bound:menu".into(),
                        ]),
                    );
                let open = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                            container,
                            menu,
                            title: &str| {
                    game.handle_packet(Clientbound::OpenScreen(OpenScreen {
                        container,
                        menu,
                        title: sunrise_core::nbt::Tag::String(title.into()),
                    }))
                    .unwrap();
                };
                open(game, 4, 0, "Chest");
                game.remote_menu_state(4, 15);
                let key = game.container.expect("chest backing");
                let generation = game.client.remote.as_ref().unwrap().menu.generation;
                let title = game.remote_menu_title().expect("chest title");

                open(game, 5, 1, "Missing factory");

                assert_eq!(game.remote_menu(), (4, CHEST.to_owned()));
                assert_eq!(game.remote_menu_title(), Some(title));
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.container, Some(key));
                assert_eq!(
                    game.world.block_entities.get(&key).unwrap().slots().len(),
                    27
                );
                let remote = game.client.remote.as_ref().unwrap();
                assert_eq!(remote.menu.generation, generation);
                assert_eq!(remote.state_id, 15);
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_moved_slot_takes_its_place_for_drawing_and_input_from_the_next_frame() {
    let mods: mod_api::Mods = Vec::<mod_api::ModEntry>::new().into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let generation = seed_native_menu(game, 1, MENU);
            game.client.settings.gui_scale = 1;
            game.client.recipe_ui.0.test_viewport = Some([1280, 720]);
            game.client.screen = Screen::Inventory;
            game.client.title.0 = None;
            game.container = Some((i32::MIN, 0, 0));
            game.client.mod_screen.0 = Some(OpenNativeScreen {
                menu: OpenNativeMenu {
                    key: (i32::MIN, 0, 0),
                    window: 1,
                    generation,
                    name: MENU.into(),
                    title: None,
                    block: Some(STONE.into()),
                    pos: None,
                    mod_type: None,
                    data: Vec::new(),
                },
                slot_moves: Default::default(),
                layout_revision: 0,
                shown: true,
                detached: false,
                screen: mod_api::__private::OwnedMenuScreen::new(
                    "geometry",
                    0,
                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![
                            Some(mod_api::ScreenSlot {
                                position: [8.0, 18.0],
                                ..Default::default()
                            }),
                            Some(mod_api::ScreenSlot {
                                position: [26.0, 18.0],
                                ..Default::default()
                            }),
                            None,
                        ],
                        inventory_y: 84.0,
                        inventory_x: 8.0,
                        ..Default::default()
                    }),
                ),
            });
            game.build_screen();
            let before = geometry(game);
            // Origin (552, 277): slot 0's 18 by 18 box starts one pixel up and left of it.
            assert_eq!(before.0, (552.0, 277.0));
            assert!(before.1.contains(&(300, [559.0, 294.0, 18.0, 18.0])));
            let batch = sunrise_client::hud::HudBatch::new(
                [1280, 720],
                game.client.settings.gui_scale_inputs(),
            );
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), None);

            // A render hook moves it after this frame's build: drawing and
            // hit-testing both still use the old place this frame.
            assert!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])).is_ok());
            assert_eq!(game.slot_position(300), (8.0, 18.0));
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), None);
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            assert_eq!(geometry(game), before);
            // The next frame's build takes it for both.
            game.build_screen();
            assert_eq!(game.slot_position(300), (48.0, 60.0));
            let after = geometry(game);
            assert!(after.1.contains(&(300, [599.0, 336.0, 18.0, 18.0])));
            // The 18 by 18 box holds the pointer at its first pixel and not past its last.
            assert_eq!(game.slot_at(&batch, 599.0, 336.0), Some(300));
            assert_eq!(game.slot_at(&batch, 616.0, 353.0), Some(300));
            assert_eq!(game.slot_at(&batch, 617.0, 340.0), None);
            assert_ne!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            assert_eq!(game.slot_position(301), (26.0, 18.0));

            // The controller's slot targets read the same committed position.
            let pad: Vec<[f32; 4]> = game
                .pad_slot_targets(&batch)
                .iter()
                .map(|t| t.rect)
                .collect();
            assert!(pad.contains(&[600.0, 337.0, 16.0, 16.0]));
            assert!(!pad.contains(&[560.0, 295.0, 16.0, 16.0]));

            // Repeating the position, or the declared one, rebuilds nothing.
            let builds =
                sunrise_client_gameplay::game::client_side::container_layout::LAYOUT_CONSTRUCTIONS
                    .with(|c| c.get());
            assert!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])).is_ok());
            assert!(game.set_screen_slot(MENU, 1, Some([26.0, 18.0])).is_ok());
            game.build_screen();
            assert_eq!(
                sunrise_client_gameplay::game::client_side::container_layout::LAYOUT_CONSTRUCTIONS
                    .with(|c| c.get()),
                builds
            );
            assert_eq!(
                game.set_screen_slot(MENU, 39, Some([1.0, 1.0])),
                Err(mod_api::ScreenSlotRefusal::NoSuchSlot)
            );
            assert_eq!(geometry(game), after);

            // Overlaps resolve by menu index: the container slot over the hotbar's
            // first slot (8, 142) and over the main inventory's first (8, 84).
            assert!(game.set_screen_slot(MENU, 0, Some([8.0, 142.0])).is_ok());
            assert!(game.set_screen_slot(MENU, 1, Some([8.0, 84.0])).is_ok());
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), Some(300));
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(301));

            // A slot beyond the image takes the click instead of making it an outside click.
            assert!(game.set_screen_slot(MENU, 0, Some([200.0, 20.0])).is_ok());
            game.build_screen();
            game.client.cursor = (555.0 + 200.0, 277.0 + 22.0);
            assert_eq!(game.target_at(&batch), (Some(300), false));
            game.client.cursor = (555.0 + 230.0, 277.0 + 22.0);
            assert_eq!(game.target_at(&batch), (None, true));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_recipe_book_screen_opens_for_a_blockless_menu() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bound",
        Box::new(BoundBookMod::default()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
            with_bound_menu_type(|| {
                seed_native_menu(game, 7, "bound:menu");
                assert!(game.open_native_screen(
                    (i32::MIN, 0, 0),
                    None,
                    3,
                    Some(sunrise_protocol::menu_extra::Message {
                        window: 7,
                        pos: None,
                        data: Vec::new(),
                    }),
                ));
                let screen = game
                    .client
                    .mod_screen
                    .0
                    .as_ref()
                    .expect("bound screen stages");
                assert_eq!(
                    screen.layout().recipe_book.as_ref().unwrap().as_str(),
                    "bound:book"
                );
                assert_eq!(screen.menu.block, None);
                assert_eq!(screen.menu.pos, None);
                assert_eq!(screen.layout().slots.len(), 3);
                assert!(screen.layout().slots[2].is_none());
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_reopened_window_uses_only_its_new_screen_instance() {
    use sunrise_protocol::{
        menu_extra::{self, CHANNEL, LIVE_DATA_REVISION},
        packets::{
            configuration::CustomPayload,
            play::{Clientbound as C, ContainerClose, OpenScreen, Serverbound as S},
        },
        registry::Ids,
    };

    let trace = ScreenTrace::default();
    let mods = vec![mod_api::ModEntry::new(
        "reopened",
        Box::new(ReopenedMod(trace.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_level::containers::with_mod_menu_types(
            [(
                REOPENED_MENU.into(),
                "reopened".into(),
                0,
                0,
                vec![],
                vec![],
                None,
                None,
                false,
                None,
            )],
            || {
                sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                    let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                    let remote = game.client.remote.as_mut().unwrap();
                    Arc::make_mut(&mut remote.registries).menu_extra_revision = LIVE_DATA_REVISION;
                    Arc::make_mut(&mut remote.builtin).registries.insert(
                        "minecraft:menu".into(),
                        Ids::from_names(vec![REOPENED_MENU.into()]),
                    );
                    let open = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                marker: i32| {
                        game.handle_packet(C::CustomPayload(CustomPayload {
                            channel: CHANNEL.into(),
                            data: menu_extra::encode(&menu_extra::Message {
                                window: 23,
                                pos: None,
                                data: marker.to_be_bytes().to_vec(),
                            })
                            .unwrap(),
                        }))
                        .unwrap();
                        game.handle_packet(C::OpenScreen(OpenScreen {
                            container: 23,
                            menu: 0,
                            title: sunrise_core::nbt::Tag::String("Reopened".into()),
                        }))
                        .unwrap();
                        game.shown_native_screen()
                            .expect("screen is shown")
                            .menu
                            .generation
                    };
                    let first = open(game, 301);
                    assert_eq!(trace.lock().unwrap().clone(), [("open", 23, 301)]);
                    assert_ne!(open(game, 742), first);
                    assert_eq!(
                        trace.lock().unwrap().clone(),
                        [("open", 23, 301), ("drop", 23, 301), ("open", 23, 742)]
                    );

                    let mut batch = HudBatch::scratch([512, 512]);
                    let (ox, oy) = game.inventory_origin(&batch);
                    game.client.cursor = (ox + 17.0, oy + 25.0);
                    assert!(game.draw_native_screen(&mut batch, ox, oy));
                    assert!(game.native_screen_press_at(&batch, true));
                    let packet = (0..32)
                        .find_map(|_| {
                            let (id, body) = peer.recv().ok()?;
                            match S::decode(id, &body).unwrap() {
                                S::ContainerButtonClick(click) => Some(click),
                                _ => None,
                            }
                        })
                        .expect("the replacement sent its menu button");
                    assert_eq!((packet.container, packet.button), (23, 742));
                    let mut lines = Vec::new();
                    game.native_screen_tooltip_lines(&batch, None, &mut lines);
                    assert_eq!(
                        lines,
                        [(sunrise_game_data::text::Text::literal("742"), u32::MAX)]
                    );

                    game.handle_packet(C::ContainerClose(ContainerClose { container: 23 }))
                        .unwrap();
                    assert!(!game.draw_native_screen(&mut batch, ox, oy));
                    assert!(!game.native_screen_press_at(&batch, true));
                    lines.clear();
                    game.native_screen_tooltip_lines(&batch, None, &mut lines);
                    assert!(lines.is_empty());
                    assert_eq!(
                        trace.lock().unwrap().clone(),
                        [
                            ("open", 23, 301),
                            ("drop", 23, 301),
                            ("open", 23, 742),
                            ("draw", 23, 742),
                            ("press", 23, 742),
                            ("tooltip", 23, 742),
                            ("drop", 23, 742),
                        ]
                    );
                })
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn authoritative_custom_book_rebuilds_cached_geometry_without_observe_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("geometry", Box::new(BookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let key = KEY.get().unwrap().clone();
            // The lightweight Game fixture does not run the launcher/content
            // bootstrap that publishes the negotiated menu registry.
            mod_api::__private::resolve_menus(["geometry:menu"]).unwrap();
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Geometry").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.joined = true;
            let generation = seed_native_menu(game, 1, "geometry:menu");
            // A registered book alone must not authorize the private protocol.
            receive(game, Message::Ready);
            assert!(!game.native_net.book_ready);
            game.client.remote.as_mut().unwrap().custom_books = true;
            game.client.settings.gui_scale = 1;
            // The fixture has no renderer; layout still needs the same viewport
            // used by geometry() to evaluate toggle gutters and book placement.
            game.client.recipe_ui.0.test_viewport = Some([1280, 720]);
            game.client.screen = Screen::Inventory;
            game.client.title.0 = None;
            game.container = Some((i32::MIN, 0, 0));
            game.client.mod_screen.0 = Some(OpenNativeScreen {
                menu: OpenNativeMenu {
                    key: (i32::MIN, 0, 0),
                    window: 1,
                    generation,
                    name: "geometry:menu".into(),
                    title: None,
                    block: Some("minecraft:stone".into()),
                    pos: None,
                    mod_type: None,
                    data: Vec::new(),
                },
                slot_moves: Default::default(),
                layout_revision: 0,
                shown: true,
                detached: false,
                screen: mod_api::__private::OwnedMenuScreen::new(
                    "geometry",
                    0,
                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            ..Default::default()
                        })],
                        inventory_y: 84.0,
                        inventory_x: 8.0,
                        title_label: None,
                        inventory_label: None,
                        own_labels: false,
                        own_background: false,
                        recipe_book: Some(key.clone()),
                        recipe_book_toggle: None,
                        recipe_book_big_result: false,
                    }),
                ),
            });
            // A snapshot may precede the first draw. The build itself must
            // collect declarations, not wait for the render refresh path.
            let recipes = sunrise_sim::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &sunrise_world::loot::PackOverlay::NONE,
                vec![(
                    "geometry:recipe".into(),
                    serde_json::json!({"type": "geometry:crafting"}),
                )],
                game.res.core.item_data.context(),
            )
            .unwrap();
            game.recipe_tables.install_test_recipes(recipes);
            game.recipe_tables.books = None;
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 6,
                    revision: 0,
                    entries: vec![(
                        ResourceId::from_registered_book(&key),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            if let Err(error) = game
                .recipe_tables
                .books
                .as_ref()
                .expect("build collected declarations")
            {
                panic!("declaration refused: {error:?}");
            }
            assert_eq!(geometry(game).0, (629.0, 277.0));
            game.reset_book_view();
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 0,
                    entries: vec![(
                        ResourceId::from_registered_book(&key),
                        BookFlags {
                            open: false,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            let closed = geometry(game);
            assert_eq!(closed.0, (552.0, 277.0));
            assert!(closed.1.contains(&(300, [559.0, 294.0, 18.0, 18.0])));
            let saved = game.level.recipe_book.custom.clone();
            assert!(game.request_custom_book_settings(
                &key,
                mod_api::RecipeBookSettings {
                    open: true,
                    filtering: true
                }
            ));
            assert_eq!(geometry(game), closed);
            assert_eq!(game.level.recipe_book.custom, saved);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 1,
                    key: ResourceId::from_registered_book(&key),
                    flags: BookFlags {
                        open: true,
                        filtering: true,
                    },
                },
            );
            let opened = geometry(game);
            assert_eq!(opened.0, (629.0, 277.0));
            assert!(opened.1.contains(&(300, [636.0, 294.0, 18.0, 18.0])));
            assert_ne!(opened.1, closed.1);
            assert_eq!(opened.1.len(), closed.1.len());
            let shift = opened.0.0 - closed.0.0;
            assert_eq!(opened.0.1, closed.0.1);
            for ((id, before), (after_id, after)) in closed.1.iter().zip(&opened.1) {
                assert_eq!(id, after_id);
                assert_eq!(*after, [before[0] + shift, before[1], before[2], before[3]]);
            }
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 2,
                    key: ResourceId::from_registered_book(&key),
                    flags: BookFlags {
                        open: false,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 3,
                    key: ResourceId::from_registered_book(&key),
                    flags: BookFlags {
                        open: true,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), opened);
            // Removed retry/busy tags and stale/gap updates leave established
            // good authority and presentation alone without latching later valid input out.
            game.remote_book_settings(&[4]);
            game.remote_book_settings(&[7]);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 9,
                    key: ResourceId::from_registered_book(&key),
                    flags: BookFlags {
                        open: false,
                        filtering: false,
                    },
                },
            );
            assert!(game.native_net.book_view.ready);
            assert_eq!(geometry(game), opened);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 4,
                    key: ResourceId::from_registered_book(&key),
                    flags: BookFlags {
                        open: false,
                        filtering: true,
                    },
                },
            );
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 5,
                    entries: vec![(
                        ResourceId::from_registered_book(&key),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            assert_eq!(geometry(game), opened);
            game.reset_book_view();
            assert_eq!(geometry(game), closed);
        });
    });
}

#[test]
#[ignore = "daily"]
fn entering_inventory_requests_catalogue_for_preinstalled_bound_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bound",
        Box::new(BoundBookMod::default()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_bound_menu_type(|| {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
                let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Bound").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                remote.custom_books = true;
                remote.joined = true;
                seed_native_menu(game, 1, "bound:menu");
                let key = (i32::MIN, 0, 0);
                game.client.screen = Screen::Playing;
                game.container = Some(key);
                assert!(game.open_native_screen(key, None, 3, None));
                let staged = game
                    .client
                    .mod_screen
                    .0
                    .as_ref()
                    .expect("bound screen staged");
                assert_eq!(staged.menu.block, None);
                let nonce = staged.layout_revision;

                // Negotiation completed before the user opened the machine. As in
                // remote_open_screen, the layout exists before Inventory is shown.
                use sunrise_sim::recipe_book::placement_sync;
                game.native_net.placement.client.hello(6);
                assert!(
                    game.native_net
                        .placement
                        .client
                        .ready(6, placement_sync::schema())
                );
                game.ensure_mod_book_catalog();
                assert!(game.native_net.placement.client.window().is_none());
                game.set_screen(Screen::Inventory);
                let window = game
                    .native_net
                    .placement
                    .client
                    .window()
                    .expect("catalogue requested on opening");
                assert_eq!(window.menu.as_str(), "bound:menu");
                assert_eq!(window.nonce, nonce);
                game.set_screen(Screen::Playing);
                assert!(game.native_net.placement.client.window().is_none());
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn non_sunrise_brand_connection_negotiates_and_brand_only_connection_cannot_place() {
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::{
        lan::{Out, PlayerSender},
        recipe_book::{
            placement_sync::{Frame, Refusal, schema},
            settings_codec,
            settings_sync::CHANNEL,
        },
    };

    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bound",
        Box::new(BoundBookMod::default()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let (tx12, rx12) = std::sync::mpsc::channel();
            let (tx13, rx13) = std::sync::mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.brand = "other-client".into();
                    guest.sender = PlayerSender::new(tx12, Arc::default());
                });
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.brand = "sunrise".into();
                    guest.sender = PlayerSender::new(tx13, Arc::default());
                });
            let drain = |rx: &std::sync::mpsc::Receiver<Out>| -> Vec<Message> {
                let mut messages = Vec::new();
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("unexpected disconnect"),
                    };
                    for body in bodies {
                        let mut reader = sunrise_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let Ok(C::CustomPayload(p)) = C::decode(id, reader.rest())
                            && p.channel == CHANNEL
                        {
                            messages.push(settings_codec::decode(&p.data).unwrap());
                        }
                    }
                }
                messages
            };
            let send = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                        conn: u32,
                        message: &Message| {
                game.guest_book_settings(conn, &settings_codec::encode(message).unwrap());
            };
            let book = ResourceId::try_from("bound:book".to_owned()).unwrap();

            // Guest 13 has brand "sunrise" only, with no book-channel capability or offer:
            // placement frames are dropped and never install server placement state.
            send(
                game,
                13,
                &Message::Placement(Frame::Hello {
                    epoch: 1,
                    schema: schema(),
                }),
            );
            assert!(drain(&rx13).is_empty());
            assert!(!game.native_net.placement.servers.contains_key(&13));

            // Guest 12 has a non-Sunrise brand, yet negotiates capability, offer and placement.
            send(game, 12, &Message::Hello);
            assert_eq!(drain(&rx12), vec![Message::Ready]);
            send(game, 12, &Message::Offer(vec![book.clone()]));
            let replies = drain(&rx12);
            let [
                Message::Snapshot {
                    epoch,
                    revision,
                    entries,
                },
            ] = replies.as_slice()
            else {
                panic!("expected initial snapshot for negotiated non-Sunrise guest");
            };
            let epoch = *epoch;
            assert_eq!(*revision, 0);
            assert_eq!(entries, &[(book.clone(), BookFlags::default())]);

            // Refused settings leave the connection usable for later valid intent.
            send(
                game,
                12,
                &Message::Request {
                    epoch: epoch + 99,
                    number: 1,
                    key: book.clone(),
                    flags: BookFlags {
                        open: true,
                        filtering: false,
                    },
                },
            );
            assert!(matches!(
                drain(&rx12).as_slice(),
                [Message::Snapshot {
                    epoch: snap_epoch,
                    revision: 0,
                    ..
                }] if *snap_epoch == epoch
            ));
            send(
                game,
                12,
                &Message::Request {
                    epoch,
                    number: 1,
                    key: book.clone(),
                    flags: BookFlags {
                        open: true,
                        filtering: true,
                    },
                },
            );
            assert_eq!(
                drain(&rx12),
                vec![Message::Update {
                    epoch,
                    revision: 1,
                    key: book.clone(),
                    flags: BookFlags {
                        open: true,
                        filtering: true,
                    },
                }]
            );

            // Placement schema refusal followed by valid negotiation works without retries.
            send(
                game,
                12,
                &Message::Placement(Frame::Hello {
                    epoch,
                    schema: schema() ^ 1,
                }),
            );
            assert_eq!(
                drain(&rx12),
                vec![Message::Placement(Frame::Refused {
                    epoch,
                    number: 0,
                    window: 0,
                    nonce: 0,
                    reason: Refusal::Unsupported,
                })]
            );
            send(
                game,
                12,
                &Message::Placement(Frame::Hello {
                    epoch,
                    schema: schema(),
                }),
            );
            assert_eq!(
                drain(&rx12),
                vec![Message::Placement(Frame::Ready {
                    epoch,
                    schema: schema(),
                })]
            );
            assert!(game.native_net.placement.servers.contains_key(&12));

            // An unplaced guest cannot negotiate regardless of brand.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.placed = false;
                });
            send(game, 13, &Message::Hello);
            assert!(drain(&rx13).is_empty());
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;
