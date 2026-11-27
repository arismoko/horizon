use super::*;
use sunrise_core::geometry::Vec3;
use winit::keyboard::KeyCode;

#[test]
#[ignore = "daily"]
fn the_inventory_poll_releases_live_queues_only_when_it_opens_a_screen() {
    with_pressing_game(|game, packets, _| {
        game.selected = 0;
        for key in [
            KeyCode::KeyQ,
            KeyCode::Digit3,
            KeyCode::Digit2,
            KeyCode::Digit2,
            KeyCode::KeyE,
            KeyCode::KeyF,
        ] {
            game.key_pressed(key, key).unwrap();
        }
        assert_eq!((game.client.screen, game.selected), (Screen::Playing, 0));
        assert!(sent(packets).is_empty());
        game.remote_handle_input();
        assert_eq!((game.client.screen, game.selected), (Screen::Inventory, 2));
        assert!(
            sent(packets).is_empty(),
            "ordinary E released swap and drop before they could act"
        );
        assert_eq!(game.pending_mapping_clicks("key.hotbar.2"), 0);

        game.set_screen(Screen::Playing);
        game.remote_send_carried();
        sent(packets);
        let mut boat = sunrise_gameplay_common::game::boats_common::Boat::new(
            47,
            "oak".into(),
            Vec3::ZERO,
            0.0,
        );
        boat.chest = true;
        game.boats.push(boat);
        game.riding = Some(47);
        for key in [
            KeyCode::KeyQ,
            KeyCode::Digit3,
            KeyCode::Digit2,
            KeyCode::Digit2,
            KeyCode::KeyE,
            KeyCode::KeyF,
        ] {
            game.key_pressed(key, key).unwrap();
        }
        assert!(sent(packets).is_empty());
        game.remote_handle_input();
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(game.selected, 2);
        assert_eq!(game.pending_mapping_clicks("key.hotbar.2"), 1);
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_command",
                "minecraft:player_action",
                "minecraft:player_action"
            ]
        );
        game.remote_handle_input();
        assert_eq!(game.selected, 1);
    });
}
