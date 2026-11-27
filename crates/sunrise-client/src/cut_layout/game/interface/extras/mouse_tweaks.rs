#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
#[path = "mouse_tweaks_result_tests.rs"]
mod result_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::client_side::extras::mouse_tweaks::Drag;
    use sunrise_gameplay_common::game::Screen;

    #[test]
    #[ignore = "daily"]
    fn a_drag_whose_button_went_up_or_was_pressed_again_unseen_moves_nothing_and_ends() {
        with_client_game(|game| {
            game.client.screen = Screen::Inventory;
            let stone = ItemStack::new("minecraft:stone", 5);
            game.carried = Some(stone.clone());
            game.client.tweaks.drag = Some(Drag {
                right: true,
                held: Some(stone.clone()),
                active: true,
                visited: vec![9],
                press: game.client.physical_presses,
            });
            // No button is held: another handler took the release this drag never heard.
            assert!(!game.tweak_drag_move(Some(10)));
            assert!(game.client.tweaks.drag.is_none());
            // Held again, but by a newer press that another handler took: still over.
            game.client.tweaks.drag = Some(Drag {
                right: true,
                held: Some(stone.clone()),
                active: true,
                visited: vec![9],
                press: game.client.physical_presses,
            });
            game.client
                .physical_buttons
                .insert(winit::event::MouseButton::Right);
            game.client.physical_presses += 1;
            assert!(!game.tweak_drag_move(Some(10)));
            assert!(game.client.tweaks.drag.is_none());
            assert!(game.inventory[10].is_none());
            assert_eq!(game.carried, Some(stone));
        });
    }
}
