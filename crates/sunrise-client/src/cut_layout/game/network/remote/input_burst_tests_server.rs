use super::*;
use crate::game::remote::interaction_event_tests::mapping_press_tests::input_burst_tests::counts;
#[cfg(feature = "client")]
use winit::event::MouseButton;
#[cfg(feature = "client")]
use winit::keyboard::KeyCode;

/// Click counters are not capped at 1024 entries: 1100 drops, 1100 attack clicks and 1100 hotbar
/// clicks are all recorded; the next tick drains all 1100 drops and attacks, while hotbar clicks
/// drain one per selection tick.
#[test]
#[ignore = "daily"]
fn a_large_burst_is_not_capped_at_1024() {
    with_pressing_game(|game, packets, triggered| {
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        game.set_test_forced_hit(None);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 64));
        let attack = game.vanilla_key_mapping("key.attack").unwrap();
        let drop = game.vanilla_key_mapping("key.drop").unwrap();
        let hotbar = game.vanilla_key_mapping("key.hotbar.2").unwrap();
        for _ in 0..1100 {
            assert_eq!(drop.click(game), Ok(()));
            assert_eq!(attack.click(game), Ok(()));
            assert_eq!(hotbar.click(game), Ok(()));
        }
        assert_eq!(game.pending_mapping_clicks("key.drop"), 1100);
        assert_eq!(game.pending_mapping_clicks("key.hotbar.2"), 1100);
        let mut acted = [0; 3];
        for _ in 0..17 {
            game.remote_handle_input();
            let tick = counts(&sent(packets));
            acted[0] += tick[0];
            acted[1] += tick[1];
        }
        acted[2] = triggered.lock().unwrap().len();
        assert_eq!(acted, [1100, 1100, 1100]);
        assert_eq!(game.pending_mapping_clicks("key.hotbar.2"), 1100 - 17);
    });
}

/// A mod's loop queues 150 attack clicks, 150 drops and five number keys before a tick: the first
/// tick drains every attack and drop click (`while (key.consumeClick())`). Each tick takes one
/// click of each number key, key 1 first (`if (keyHotbarSlots[i].consumeClick())`), so 7, 7, 4,
/// 4, 4 select slot 7 for two ticks, then 4.
#[test]
#[ignore = "daily"]
fn a_mods_burst_drains_all_clicks_in_one_tick() {
    with_pressing_game(|game, packets, triggered| {
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        game.set_test_forced_hit(None);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 64));
        let attack = game.vanilla_key_mapping("key.attack").unwrap();
        let drop = game.vanilla_key_mapping("key.drop").unwrap();
        let hotbar_7 = game.vanilla_key_mapping("key.hotbar.7").unwrap();
        let hotbar_4 = game.vanilla_key_mapping("key.hotbar.4").unwrap();
        for _ in 0..150 {
            assert_eq!(attack.click(game), Ok(()));
            assert_eq!(drop.click(game), Ok(()));
        }
        for mapping in [hotbar_7, hotbar_7, hotbar_4, hotbar_4, hotbar_4] {
            assert_eq!(mapping.click(game), Ok(()));
        }
        let mut ticks = Vec::new();
        let mut selected = Vec::new();
        for _ in 0..4 {
            game.remote_handle_input();
            ticks.push(counts(&sent(packets)));
            selected.push(game.selected);
        }
        assert_eq!(ticks, [[150, 150, 1], [0, 0, 0], [0, 0, 0], [0, 0, 0]]);
        assert_eq!(selected, [6, 6, 3, 3]);
        assert_eq!(triggered.lock().unwrap().len(), 150);
    });
}

/// A player's burst between two ticks, keys and buttons: the number key selects at the tick's
/// pick, then `handleKeybinds` swaps before it drops, whatever the press order, and both before
/// the click's punch; the first drop tells the server of the new slot
/// (`ensureHasSentCarriedItem`), the swap doesn't. One press of each a tick for twenty ticks acts
/// once a tick, on its own tick, the drop before the punch.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_players_burst_acts_on_the_next_tick_in_handle_keybinds_order() {
    with_pressing_game(|game, packets, _| {
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        game.set_test_forced_hit(None);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 64));
        game.key_pressed(KeyCode::Digit2, KeyCode::Digit2).unwrap();
        game.play_mouse(MouseButton::Left, true).unwrap();
        game.play_mouse(MouseButton::Left, false).unwrap();
        game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
        game.key_pressed(KeyCode::KeyF, KeyCode::KeyF).unwrap();
        game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "nothing before the tick"
        );
        game.remote_handle_input();
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_action",
                "minecraft:set_carried_item",
                "minecraft:player_action",
                "minecraft:player_action",
                "minecraft:punch"
            ]
        );
        assert_eq!(game.selected, 1);

        for _ in 0..20 {
            game.play_mouse(MouseButton::Left, true).unwrap();
            game.play_mouse(MouseButton::Left, false).unwrap();
            game.key_pressed(KeyCode::KeyQ, KeyCode::KeyQ).unwrap();
            game.remote_handle_input();
            assert_eq!(
                sent(packets),
                ["minecraft:player_action", "minecraft:punch"]
            );
        }
    });
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
