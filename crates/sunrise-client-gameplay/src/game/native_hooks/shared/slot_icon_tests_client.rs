use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use sunrise_gameplay_common::game::native_hooks::slot_icon_tests::ICON_MENU;
use sunrise_gameplay_common::game::native_hooks::slot_icon_tests::IconMod;

#[test]
#[ignore = "daily"]
fn empty_slot_draws_icon_but_a_filled_slot_does_not() {
    mod_platform::testing::with_mods(mod_api::Mods::default(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let icon = "minecraft:container/slot/shield";
            let Some(sunrise_client::hud::GuiSprite::Present { rect, .. }) =
                game.client.gui.atlas.gui_lookup(icon)
            else {
                panic!("the shield icon must resolve to a real GUI sprite");
            };
            let generation = seed_native_menu(game, 1, "test:menu");
            game.client.screen = Screen::Inventory;
            game.container = Some((1, 2, 3));
            game.client.mod_screen.0 = Some(OpenNativeScreen {
                menu: OpenNativeMenu {
                    key: (1, 2, 3),
                    window: 1,
                    generation,
                    name: "test:menu".into(),
                    title: None,
                    block: Some("test:block".into()),
                    pos: None,
                    mod_type: None,
                    data: Vec::new(),
                },
                layout_revision: 0,
                slot_moves: Default::default(),
                shown: true,
                detached: false,
                screen: mod_api::__private::OwnedMenuScreen::new(
                    "test",
                    0,
                    mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            no_item_icon: Some(icon.into()),
                        })],
                        ..Default::default()
                    }),
                ),
            });
            let mut actual = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut actual, None, 300, &None, 8.0, 18.0);
            let mut expected = HudBatch::scratch([128, 128]);
            expected.rect_uv(8.0, 18.0, 16.0, 16.0, rect, 0xFFFF_FFFF);
            let geometry = |batch: &HudBatch| {
                batch
                    .vertices
                    .iter()
                    .map(|v| (v.pos, v.uv))
                    .collect::<Vec<_>>()
            };
            assert_eq!(geometry(&actual), geometry(&expected));
            let filled = Some(ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            });
            let mut with_icon = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut with_icon, None, 300, &filled, 8.0, 18.0);
            game.client
                .mod_screen
                .0
                .as_mut()
                .unwrap()
                .layout_mut()
                .slots[0]
                .as_mut()
                .unwrap()
                .no_item_icon = None;
            let mut without_icon = HudBatch::scratch([128, 128]);
            game.draw_slot_item(&mut without_icon, None, 300, &filled, 8.0, 18.0);
            assert_eq!(geometry(&with_icon), geometry(&without_icon));
        });
    });
}

#[test]
#[ignore = "daily"]
fn unknown_slot_icon_opens_screen_and_draws_missing_sprite() {
    // Content registries are process-global.
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        "game::native_hooks::slot_icon_tests::unknown_slot_icon_opens_screen_and_draws_missing_sprite",
        "SUNRISE_SLOT_ICON_OPEN_CHILD",
        &["open"],
    ) {
        return;
    }
    let unknown = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "icon_open",
        Box::new(IconMod(unknown.clone())),
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
                ICON_MENU.into(),
                "icon_open".into(),
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
                crate::game::native_entity_refs::tests::with_game_role_client(
                    root.prepared().to_owned(),
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    |game| {
                        seed_native_menu(game, 1, ICON_MENU);
                        let pos = (1, 64, 2);
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                        let block = game
                            .pack
                            .assets
                            .registry
                            .default_state("icon_open:machine")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, block).unwrap();
                        assert!(game.open_native_screen(pos, Some(pos), 9, None));
                        assert!(
                            game.client.mod_screen.0.is_some(),
                            "valid icon must open the screen"
                        );
                        unknown.store(true, Ordering::Relaxed);
                        seed_native_menu(game, 2, ICON_MENU);
                        assert!(
                            game.open_native_screen(pos, Some(pos), 9, None),
                            "unknown icon must open and fall back to the missing sprite"
                        );
                        game.container = Some(pos);
                        game.set_screen(Screen::Inventory);
                        let mut actual = HudBatch::scratch([128, 128]);
                        game.draw_slot_item(&mut actual, None, 300, &None, 8.0, 18.0);
                        let missing = game
                            .client
                            .gui
                            .atlas
                            .gui_sprite_or_missing("icon_open:missing")
                            .expect("missing sprite in atlas");
                        assert_eq!(
                            missing,
                            game.client.gui.atlas.sprites[sunrise_client::hud::MISSING_SPRITE]
                        );
                        let mut expected = HudBatch::scratch([128, 128]);
                        expected.rect_uv(8.0, 18.0, 16.0, 16.0, missing, 0xFFFF_FFFF);
                        let geometry = |batch: &HudBatch| {
                            batch
                                .vertices
                                .iter()
                                .map(|v| (v.pos, v.uv))
                                .collect::<Vec<_>>()
                        };
                        assert!(!expected.vertices.is_empty());
                        assert_eq!(geometry(&actual), geometry(&expected));
                    },
                );
            },
        );
    });
}
