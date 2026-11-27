use sunrise_client::hud::HudBatch;
use sunrise_client::hud::rgba;
use sunrise_client_gameplay::game::native_hooks::{
    OpenNativeMenu, OpenNativeScreen, seed_native_menu,
};
use sunrise_gameplay_common::game::Screen;

#[test]
#[ignore = "daily"]
fn inventory_label_default_follows_image_height_not_inventory_slots() {
    mod_platform::testing::with_mods(mod_api::Mods::default(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "labels").unwrap();
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
                        own_background: true,
                        ..Default::default()
                    }),
                ),
            });
            let (ox, oy) = (17.0, 23.0);
            for (height, inventory_y, label_y) in [
                (166.0, 84.0, 72.0), // Standard container.
                (133.0, 51.0, 39.0), // Hopper.
                (168.0, 85.0, 74.0), // Three-row chest.
                (167.0, 84.0, 73.0), // Shulker box.
                (166.0, 30.0, 72.0), // Moving slots does not move the label.
            ] {
                let screen = game.client.mod_screen.0.as_mut().unwrap().layout_mut();
                screen.height = height;
                screen.inventory_y = inventory_y;
                let mut actual = HudBatch::scratch([512, 512]);
                assert!(game.draw_native_screen(&mut actual, ox, oy));
                let mut expected = HudBatch::scratch([512, 512]);
                expected.text(
                    &game.client.gui.atlas,
                    &game.translate_with_fallback("container.inventory", "Inventory"),
                    ox + 8.0,
                    oy + label_y,
                    rgba(64, 64, 64, 255),
                    false,
                );
                assert!(!expected.vertices.is_empty());
                let geometry = |batch: &HudBatch| {
                    batch
                        .vertices
                        .iter()
                        .map(|v| (v.pos, v.uv))
                        .collect::<Vec<_>>()
                };
                assert_eq!(geometry(&actual), geometry(&expected), "height {height}");
            }
            // A server title lets the actual drawing path exercise both labels.
            let title = sunrise_game_data::text::Text::literal("Title");
            game.client
                .remote
                .as_mut()
                .unwrap()
                .set_menu_title(Some(title.clone()));
            for (title_at, inventory_at, title_expected, inventory_expected, own_labels) in [
                (None, None, [8.0, 6.0], [8.0, 72.0], false),
                (Some([44.0, 15.0]), None, [44.0, 15.0], [8.0, 72.0], false),
                (None, Some([107.0, 41.0]), [8.0, 6.0], [107.0, 41.0], false),
                (
                    Some([44.0, 15.0]),
                    Some([107.0, 41.0]),
                    [44.0, 15.0],
                    [107.0, 41.0],
                    true,
                ),
            ] {
                let screen = game.client.mod_screen.0.as_mut().unwrap().layout_mut();
                screen.title_label = title_at;
                screen.inventory_label = inventory_at;
                screen.own_labels = own_labels;
                let mut actual = HudBatch::scratch([512, 512]);
                assert!(game.draw_native_screen(&mut actual, ox, oy));
                let mut expected = HudBatch::scratch([512, 512]);
                if !own_labels {
                    expected.text_styled(
                        &game.client.gui.atlas,
                        &title,
                        ox + title_expected[0],
                        oy + title_expected[1],
                        rgba(64, 64, 64, 255),
                        false,
                    );
                    expected.text(
                        &game.client.gui.atlas,
                        &game.translate_with_fallback("container.inventory", "Inventory"),
                        ox + inventory_expected[0],
                        oy + inventory_expected[1],
                        rgba(64, 64, 64, 255),
                        false,
                    );
                }
                let geometry = |batch: &HudBatch| {
                    batch
                        .vertices
                        .iter()
                        .map(|v| (v.pos, v.uv))
                        .collect::<Vec<_>>()
                };
                assert_eq!(geometry(&actual), geometry(&expected));
            }
        });
    });
}
