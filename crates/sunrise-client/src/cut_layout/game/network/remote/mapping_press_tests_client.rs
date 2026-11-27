use super::*;
use crate::game::remote::interaction_event_tests::mapping_press_tests::Heard;
use crate::game::remote::interaction_event_tests::mapping_press_tests::sent;
use crate::game::remote::interaction_event_tests::mapping_press_tests::with_pressing_game;
use mod_api::VanillaKeyMappingRef;
use std::sync::{Mutex, OnceLock};
use sunrise_gameplay_common::game::Screen;

#[path = "input_burst_tests_client.rs"]
pub(crate) mod input_burst_tests;

#[test]
#[ignore = "daily"]
fn a_mod_releases_the_toolbar_mapping_while_its_physical_key_stays_down() {
    with_pressing_game(|game, _, _| {
        use winit::keyboard::KeyCode;
        let mapping = game
            .vanilla_key_mapping("key.saveToolbarActivator")
            .unwrap();
        game.client.physical_keys.insert(KeyCode::KeyC);
        game.mapped_key_event(KeyCode::KeyC, true, false).unwrap();
        assert!(game.mapped_key_down("key.saveToolbarActivator"));
        mapping.set_down(game, false).unwrap();
        assert_eq!(mapping.physical_down(game), Ok(true));
        assert_eq!(mapping.is_down(game), Ok(false));
        assert!(!game.mapped_key_down("key.saveToolbarActivator"));
    });
}

#[test]
#[ignore = "daily"]
fn opening_a_screen_releases_mod_holds() {
    with_pressing_game(|game, _, _| {
        use sunrise_client::held::Source;
        use winit::event::MouseButton;
        use winit::keyboard::{KeyCode, ModifiersState};

        let forward_mapping = game.vanilla_key_mapping("key.forward").unwrap();
        let sneak_mapping = game.vanilla_key_mapping("key.sneak").unwrap();
        let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
        let use_mapping = game.vanilla_key_mapping("key.use").unwrap();
        game.mapped_key_event(KeyCode::KeyW, true, false).unwrap();
        assert!(forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, false).unwrap();
        assert!(!forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, true).unwrap();
        game.mapped_key_event(KeyCode::KeyW, false, false).unwrap();
        assert!(!forward_mapping.is_down(game).unwrap());
        forward_mapping.set_down(game, true).unwrap();
        attack_mapping.set_down(game, true).unwrap();
        use_mapping.set_down(game, true).unwrap();
        assert!(game.mapped_key_down("key.attack") && game.mapped_key_down("key.use"));
        game.client
            .physical_buttons
            .extend([MouseButton::Left, MouseButton::Right]);
        game.set_screen(Screen::Inventory);
        // No redraw/reconciliation before the screen closes. Controller releases
        // forwarded for cleanup cannot give the hold back to the mouse in a screen.
        for button in [MouseButton::Left, MouseButton::Right] {
            game.mouse_mapping(button, false, Source::Pad).unwrap();
        }
        game.set_screen(Screen::Playing);
        assert!(!game.mapped_key_down("key.attack") && !game.mapped_key_down("key.use"));
        assert!(!forward_mapping.is_down(game).unwrap());

        // ModifiersChanged can contain an unrelated change, or be the OS's only
        // report of a real release. Only the recovered sneak release takes over.
        game.client.settings.toggle_crouch = false;
        game.client.physical_keys.insert(KeyCode::ShiftLeft);
        game.update_modifiers(ModifiersState::SHIFT | ModifiersState::ALT);
        game.mapped_key_event(KeyCode::ShiftLeft, true, false)
            .unwrap();
        sneak_mapping.set_down(game, true).unwrap();
        game.update_modifiers(ModifiersState::SHIFT);
        game.heal_hold_latches();
        assert!(sneak_mapping.is_down(game).unwrap());
        game.update_modifiers(ModifiersState::empty());
        assert!(!sneak_mapping.is_down(game).unwrap());
    });
}

pub(crate) fn with_callback_game(
    click: bool,
    test: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        &Mutex<Vec<String>>,
        &Mutex<Vec<Hand>>,
        Option<VanillaKeyMappingRef>,
    ),
) {
    let packets = Arc::new(Mutex::new(Vec::new()));
    let triggered = Arc::new(Mutex::new(Vec::new()));
    let attack_mapping = Arc::new(OnceLock::new());
    let mods = vec![mod_api::ModEntry::new(
        "presses",
        Box::new(Heard {
            packets: packets.clone(),
            triggered: triggered.clone(),
            click_in_callback: Arc::new(std::sync::atomic::AtomicBool::new(click)),
            attack_mapping: attack_mapping.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            game.set_local_mode(sunrise_command::args::GameMode::Survival);
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            let attack = click.then(|| {
                let attack = game.vanilla_key_mapping("key.attack").unwrap();
                attack_mapping.set(attack).unwrap();
                attack
            });
            // The tick's first packet, the held slot (`MultiPlayerGameMode.tick`), then its keys.
            game.remote_send_carried();
            game.remote_handle_input();
            packets.lock().unwrap().clear();
            test(game, &packets, &triggered, attack);
        })
    });
}
