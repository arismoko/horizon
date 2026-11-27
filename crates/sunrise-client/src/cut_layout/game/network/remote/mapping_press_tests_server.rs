use super::*;
use crate::game::remote::interaction_event_tests::mapping_press_tests::sent;
use crate::game::remote::interaction_event_tests::mapping_press_tests::with_pressing_game;

#[path = "input_burst_tests_server.rs"]
pub(crate) mod input_burst_tests;

/// Q in creative mode: `MultiPlayerGameMode.dropItem` sends the drop and nothing else, so the
/// end of the tick sends no creative slot edit for the stack it left (only the creative screen
/// sends those).
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_creative_drop_sends_no_creative_slot() {
    with_pressing_game(|game, packets, _| {
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        game.set_test_forced_hit(None);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 16));
        // The setup's stack is the server's as well.
        game.remote_synced();
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        assert!(
            sent.iter().any(|p| p == "minecraft:player_action"),
            "{sent:?}"
        );
        assert!(
            !sent.iter().any(|p| p == "minecraft:set_creative_mode_slot"),
            "{sent:?}"
        );
        assert_eq!(
            game.common.inventory[game.common.selected]
                .as_ref()
                .map(|s| s.count),
            Some(15)
        );
    });
}

/// A creative toolbar load and Q in one tick (`handleKeybinds`: the number keys' loop, then
/// key.drop): the load sends the new stack at once (`handleCreativeModeItemAdd`), so the drop
/// that follows drops what the server now holds, and the end of the tick sends nothing more.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_toolbar_load_goes_out_before_a_drop_in_its_tick() {
    with_pressing_game(|game, packets, _| {
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        game.set_test_forced_hit(None);
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stone", 16));
        game.remote_synced();
        // handleHotbarLoadOrSave puts the saved stack in the slot and sends it.
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:dirt", 16));
        game.remote_creative_slot_add(game.selected);
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        let at = |id: &str| sent.iter().position(|p| p == id);
        assert_eq!(
            sent.iter()
                .filter(|p| *p == "minecraft:set_creative_mode_slot")
                .count(),
            1,
            "{sent:?}"
        );
        assert!(
            at("minecraft:set_creative_mode_slot") < at("minecraft:player_action"),
            "{sent:?}"
        );
        assert_eq!(
            game.common.inventory[game.common.selected]
                .as_ref()
                .map(|s| (s.item.as_ref(), s.count)),
            Some(("minecraft:dirt", 15))
        );
    });
}

/// A click on nothing punches and sets the miss cooldown (not in creative); a second click inside
/// it does nothing.
#[test]
#[ignore = "daily"]
fn clicking_attack_on_nothing_punches_and_waits_out_the_miss_cooldown() {
    with_pressing_game(|game, packets, _| {
        let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
        game.set_test_forced_hit(None);
        assert_eq!(attack_mapping.set_down(game, true), Ok(()));
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        // 10, less the tick's own count down after `handleKeybinds`.
        assert_eq!(game.clicks.miss_time, 9);
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new(), "inside the cooldown");
        assert_eq!(game.clicks.miss_time, 8);

        game.clicks.miss_time = 0;
        game.set_local_mode(sunrise_command::args::GameMode::Creative);
        assert_eq!(attack_mapping.click(game), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        assert_eq!(game.clicks.miss_time, 0, "no cooldown in creative");
    });
}

#[cfg(feature = "client")]
pub(crate) fn stone_under_the_crosshair(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(0, 64, 0, stone).unwrap();
    game.set_test_forced_hit(Some(hit()));
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
