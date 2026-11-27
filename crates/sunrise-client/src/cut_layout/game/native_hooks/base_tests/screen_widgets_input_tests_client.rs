use super::*;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::Cancel;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::Seen;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::key;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::send;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::take;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::with_input_game;
use crate::game::native_hooks::base::screen_widgets_tests::input_routing::with_input_game_as;
use mod_api::prelude::ScreensClientHost;
use sunrise_client::{keybinds::VanillaMappingSlot, preedit, widgets};
use sunrise_client_gameplay::game::{ClientGame, WindowKeyInput};
use sunrise_gameplay_common::game::Screen;
use sunrise_items::items::ItemStack;
use winit::{dpi::PhysicalPosition, event::Ime, keyboard::PhysicalKey};

fn window_key(game: &mut ClientGame<'_>, input: WindowKeyInput<'_>) {
    game.dispatch_window_key(&NoLoop, input, |_| false);
}

#[test]
#[ignore = "daily"]
fn inventory_release_post_distinguishes_quick_craft_from_swallowed_release() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::inventory_release_post_distinguishes_quick_craft_from_swallowed_release",
        "SUNRISE_SCREEN_QUICK_CRAFT_CHILD",
        &["inventory"],
    ) {
        return;
    }
    with_input_game(|game, id, state| {
        use sunrise_level::{chunk::Chunk, containers::BlockEntity};
        let pos = (0, 0, 0);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        game.world.set(pos.0, pos.1, pos.2, chest).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(vec![None; 27]));
        game.client
            .remote
            .as_mut()
            .unwrap()
            .menu
            .set(Some((1, "minecraft:generic_9x3".to_owned())));
        game.container = Some(pos);
        game.open_player_inventory();
        game.carried = Some(ItemStack::new("minecraft:stick", 8));
        let batch = game.gui_batch().unwrap();
        let layout = game.built_container(&batch).unwrap();
        let point = |slot| {
            let &(_, [x, y, ..]) = layout.slots.iter().find(|(i, _)| *i == slot).unwrap();
            (f64::from(x + 5.0), f64::from(y + 5.0))
        };
        let first = point(300);
        let second = point(301);
        let third = point(302);
        move_mouse(game, id, first.0, first.1);
        send_mouse_button(game, id, MouseButton::Left, true);
        move_mouse(game, id, second.0, second.1);
        take(state);
        send_mouse_button(game, id, MouseButton::Left, false);
        assert_eq!(take(state), [Seen::ReleasePost("inventory".into(), false)]);
        assert!(game.carried.is_none());
        assert_eq!(game.slot_view(300).unwrap().count, 4);
        assert_eq!(game.slot_view(301).unwrap().count, 4);

        game.carried = Some(ItemStack::new("minecraft:stick", 6));
        // Start on a fresh slot so a click within 250ms does not trigger vanilla PickupAll.
        move_mouse(game, id, third.0, third.1);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert!(game.client.inv_drag.is_some());
        take(state);
        send_mouse_button(game, id, MouseButton::Right, false);
        assert_eq!(take(state), [Seen::ReleasePost("inventory".into(), true)]);
        assert!(game.client.inv_drag.is_none());
        assert_eq!(game.carried.as_ref().unwrap().count, 6);
        send_mouse_button(game, id, MouseButton::Left, false);
        assert_eq!(take(state), [Seen::ReleasePost("inventory".into(), true)]);
    });
}

#[test]
#[ignore = "daily"]
fn native_and_mounted_press_tab_and_preedit_use_the_receiving_screen() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::native_and_mounted_press_tab_and_preedit_use_the_receiving_screen",
        "SUNRISE_SCREEN_INPUT_ROUTING_CHILD",
        &["routing"],
    ) {
        return;
    }
    with_input_game(|game, id, state| {
        game.client.pause_menu = true;
        game.set_screen(Screen::Paused);
        publish_frame(game, 1);
        assert_eq!(game.mounted_widget_targets().len(), 1);
        let (w, h) = game.gui_size();
        let (x, y, _, _, _, _) = game.pause_layout(w, h).buttons[0];
        move_mouse(game, id, f64::from(x + 10.0), f64::from(y + 10.0));
        send_mouse_button(game, id, MouseButton::Left, true);
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(
            take(state),
            [Seen::PressPre, Seen::PressPost("paused".into(), true)]
        );
        send_mouse_button(game, id, MouseButton::Left, false);

        assert!(game.mod_screen_open("input_route"));
        let focused = game.client.screen_state.0.widgets.focused_id().unwrap();
        move_mouse(game, id, 24.0, 30.0);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert_eq!(
            take(state),
            [
                Seen::PressPre,
                Seen::Activation("decline"),
                Seen::PressPost("mod".into(), true)
            ]
        );
        assert_eq!(
            game.client.screen_state.0.widgets.focused_id(),
            Some(focused)
        );
        send_mouse_button(game, id, MouseButton::Left, false);
        take(state);
        move_mouse(game, id, 300.0, 100.0);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert_eq!(
            take(state),
            [Seen::PressPre, Seen::PressPost("mod".into(), false)]
        );
        assert_eq!(
            game.client.screen_state.0.widgets.focused_id(),
            Some(focused)
        );
        send_mouse_button(game, id, MouseButton::Left, false);
        game.mod_screen_close("input_route");
        assert_eq!(game.client.screen, Screen::Playing);
        take(state);

        let dialog = sunrise_command::snbt::parse_fully(
            r#"{type:"minecraft:notice",title:"Input",inputs:[{type:"minecraft:text",key:"name",label:"Name"}]}"#,
        ).unwrap();
        game.show_dialog(&dialog);
        assert!(game.dialog_open());
        publish_frame(game, 2);
        let tab = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            key(game, KeyCode::Tab, ElementState::Pressed)
        };
        tab(game); // Warning, tab group -10.
        tab(game); // Native field.
        assert!(game.dialog_typing());
        assert!(!game.client.screen_state.0.widgets.has_focus());
        tab(game); // Native notice button.
        tab(game); // Appended mod button.
        assert!(!game.dialog_typing());
        assert!(game.client.screen_state.0.widgets.has_focus());
        key(game, KeyCode::Enter, ElementState::Pressed);
        assert_eq!(take(state), [Seen::Activation("after_dialog")]);
        game.clear_dialog();

        game.begin_chat_screen("native draft", false);
        send(
            game,
            id,
            WindowEvent::Ime(Ime::Preedit("ni".into(), Some((0, 2)))),
        );
        assert!(preedit::should_suppress_key());
        assert!(take(state).is_empty());
        assert!(game.open_screen_overlay("link_prompt", |game| {
            game.client.link_prompt = Some("https://example.com".into());
        }));
        assert!(game.mounted_widget_targets().is_empty());
        send(
            game,
            id,
            WindowEvent::Ime(Ime::Preedit("ka".into(), Some((0, 2)))),
        );
        assert_eq!(
            take(state),
            [Seen::PreeditPost("link_prompt".into(), "ka".into())]
        );
        assert!(!preedit::should_suppress_key());
        assert_eq!(
            game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
            "native draft"
        );
    });
}

pub(crate) fn publish_frame(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, frame: u64) {
    let renderer = game.client.renderer.0.as_ref().unwrap();
    let size = [renderer.config.width, renderer.config.height];
    game.hud(size);
    widgets::set_action_screen(game.key_screen());
    widgets::end_frame(frame);
}

#[test]
#[ignore = "daily"]
fn release_drag_and_wheel_cancellation_preserve_input_bookkeeping() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::release_drag_and_wheel_cancellation_preserve_input_bookkeeping",
        "SUNRISE_SCREEN_PHYSICAL_INPUT_CHILD",
        &["input"],
    ) {
        return;
    }
    with_input_game(|game, id, state| {
        assert!(game.mod_screen_open("input_route"));
        let cancel = |cancel| state.lock().unwrap().cancel = cancel;
        let forward = VanillaMappingSlot::by_default_key(KeyCode::KeyW).unwrap();
        let jump = VanillaMappingSlot::by_default_key(KeyCode::Space).unwrap();
        game.client.settings.keys.set("key.jump", "key.keyboard.w");
        game.insert_device_key(KeyCode::KeyW);
        game.insert_device_key(KeyCode::Space);
        assert!(
            game.client.vanilla_mappings.is_down(forward)
                && game.client.vanilla_mappings.is_down(jump)
        );
        cancel(Cancel::ReleasePre);
        key(game, KeyCode::KeyW, ElementState::Released);
        assert_eq!(take(state), [Seen::KeyPre("key.keyboard.w".into())]);
        assert!(!game.client.keys.raw(&KeyCode::KeyW) && !game.client.keys.raw(&KeyCode::Space));
        assert!(
            !game.client.vanilla_mappings.is_down(forward)
                && !game.client.vanilla_mappings.is_down(jump)
        );

        game.insert_device_key(KeyCode::KeyW);
        send_key_control(game, "key w up");
        assert_eq!(take(state), [Seen::KeyPre("key.keyboard.w".into())]);
        assert!(game.client.keys.raw(&KeyCode::KeyW));
        cancel(Cancel::ReleasePost);
        key(game, KeyCode::KeyW, ElementState::Released);
        assert_eq!(
            take(state),
            [
                Seen::KeyPre("key.keyboard.w".into()),
                Seen::KeyPost("key.keyboard.w".into())
            ]
        );
        assert!(game.client.keys.raw(&KeyCode::KeyW));
        cancel(Cancel::None);
        send_key_control(game, "key w up");
        assert!(!game.client.keys.raw(&KeyCode::KeyW));
        take(state);

        move_mouse(game, id, 24.0, 30.0);
        cancel(Cancel::Press);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert_eq!(take(state), [Seen::PressPre]);
        assert!(game.client.physical_buttons.contains(&MouseButton::Left));
        send_mouse_button(game, id, MouseButton::Left, false);
        assert!(!game.client.physical_buttons.contains(&MouseButton::Left));
        take(state);

        move_mouse(game, id, 300.0, 100.0);
        cancel(Cancel::None);
        send_mouse_button(game, id, MouseButton::Left, true);
        send_mouse_button(game, id, MouseButton::Right, true);
        take(state);
        move_mouse(game, id, 310.0, 120.0);
        assert_eq!(
            take(state),
            [
                Seen::DragPre(1, [10.0, 20.0], [310.0, 120.0]),
                Seen::DragPost
            ]
        );
        cancel(Cancel::Drag);
        move_mouse(game, id, 312.0, 123.0);
        assert_eq!(take(state), [Seen::DragPre(1, [2.0, 3.0], [312.0, 123.0])]);
        assert_eq!(game.cursor(), [312.0, 123.0]);
        send_mouse_button(game, id, MouseButton::Left, false);
        take(state);
        move_mouse(game, id, 314.0, 126.0);
        assert!(take(state).is_empty());
        assert!(game.client.physical_buttons.contains(&MouseButton::Right));
        send_mouse_button(game, id, MouseButton::Left, true);
        game.client.physical_keys.insert(KeyCode::ShiftLeft);
        game.client.modifiers = winit::keyboard::ModifiersState::SHIFT;
        take(state);
        send(game, id, WindowEvent::Focused(false));
        move_mouse(game, id, 316.0, 128.0);
        assert!(take(state).is_empty());
        assert!(game.client.physical_buttons.is_empty() && game.client.physical_keys.is_empty());
        assert!(game.client.modifiers.is_empty() && game.client.drag_button.is_none());
        send(game, id, WindowEvent::Focused(true));
        move_mouse(game, id, 24.0, 30.0);

        cancel(Cancel::ScreenWheel);
        send_scroll(game, id, 0.25, -0.75);
        assert_eq!(take(state), [Seen::ScreenWheelPre([0.25, -0.75])]);
        cancel(Cancel::None);
        send_scroll(game, id, 0.25, -0.75);
        assert_eq!(
            take(state),
            [
                Seen::ScreenWheelPre([0.25, -0.75]),
                Seen::ScreenWheelChild([0.25, -0.75]),
                Seen::ScreenWheelPost([0.25, -0.75])
            ]
        );
        game.client.pad.0.menu.synthetic = true;
        send_scroll(game, id, 0.25, -0.75);
        game.client.pad.0.menu.synthetic = false;
        assert!(take(state).is_empty());
        game.mod_screen_close("input_route");
        assert_eq!(game.client.screen, Screen::Playing);
        game.selected = 4;
        game.client.settings.mouse_wheel_sensitivity = 1.0;
        game.client.settings.discrete_scroll = false;
        move_mouse(game, id, 100.0, 110.0);
        send_scroll(game, id, 0.25, 0.0);
        assert_eq!(game.selected, 4);
        assert_eq!(
            take(state),
            [Seen::GameWheel(
                [0.25, 0.0],
                [0, 0],
                [false; 3],
                [100.0, 110.0],
                0
            )]
        );
        send_scroll(game, id, 0.75, 0.0);
        assert_eq!(game.selected, 5);
        assert_eq!(
            take(state),
            [Seen::GameWheel(
                [0.75, 0.0],
                [1, 0],
                [false; 3],
                [100.0, 110.0],
                0
            )]
        );
        cancel(Cancel::GameWheel);
        send_scroll(game, id, 0.0, 1.0);
        assert_eq!(game.selected, 5);
        assert_eq!(
            take(state),
            [Seen::GameWheel(
                [0.0, 1.0],
                [0, 1],
                [false; 3],
                [100.0, 110.0],
                0
            )]
        );
        cancel(Cancel::None);
        send_scroll(game, id, 0.0, 0.25);
        assert_eq!(game.selected, 5);
        send_scroll(game, id, 0.0, 0.75);
        assert_eq!(game.selected, 4);
        assert_eq!(
            take(state),
            [
                Seen::GameWheel([0.0, 0.25], [0, 0], [false; 3], [100.0, 110.0], 0),
                Seen::GameWheel([0.0, 0.75], [0, 1], [false; 3], [100.0, 110.0], 0),
            ]
        );
        game.client.settings.mouse_wheel_sensitivity = 2.0;
        let pixel_wheel = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            send(
                game,
                id,
                WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(60.0, -30.0)),
                    phase: TouchPhase::Moved,
                },
            );
        };
        pixel_wheel(game);
        assert_eq!(
            take(state),
            [Seen::GameWheel(
                [4.0, -2.0],
                [4, -2],
                [false; 3],
                [100.0, 110.0],
                0
            )]
        );
        game.client.settings.discrete_scroll = true;
        pixel_wheel(game);
        assert_eq!(
            take(state),
            [Seen::GameWheel(
                [2.0, -2.0],
                [2, -2],
                [false; 3],
                [100.0, 110.0],
                0
            )]
        );

        let selected = game.selected;
        let player = game.player.take();
        send_scroll(game, id, 0.0, 1.0);
        game.player = player;
        game.sleeping = Some([0, 64, 0]);
        send_scroll(game, id, 0.0, 1.0);
        game.sleeping = None;
        assert_eq!(game.selected, selected);
        // A skipped gameplay wheel can belong to a screen, but never the hotbar hook.
        assert!(
            take(state)
                .iter()
                .all(|event| !matches!(event, Seen::GameWheel(..)))
        );
    });
}

#[test]
#[ignore = "daily"]
fn hover_moves_fire_before_drag_and_a_screen_grab_ends_with_the_screen() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::hover_moves_fire_before_drag_and_a_screen_grab_ends_with_the_screen"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_POINTER_GRAB_CHILD",
        &["grab"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, state| {
        state.lock().unwrap().record_pointer = true;
        // No screen: nothing fires.
        move_mouse(game, id, 20.0, 20.0);
        assert!(take(state).is_empty());
        assert!(game.mod_screen_open("input_route"));

        // A hover move, then a held-button move: the moved hook comes first both times.
        move_mouse(game, id, 300.0, 100.0);
        assert_eq!(take(state), [Seen::Moved([280.0, 80.0], [300.0, 100.0])]);
        move_mouse(game, id, 300.0, 100.0);
        assert!(take(state).is_empty(), "a zero delta fires nothing");
        send_mouse_button(game, id, MouseButton::Left, true);
        take(state);
        move_mouse(game, id, 310.0, 120.0);
        assert_eq!(
            take(state),
            [
                Seen::Moved([10.0, 20.0], [310.0, 120.0]),
                Seen::DragPre(0, [10.0, 20.0], [310.0, 120.0]),
                Seen::DragPost
            ]
        );
        send_mouse_button(game, id, MouseButton::Left, false);
        take(state);

        // Cancel skips the drag too, and the cursor still moves.
        state.lock().unwrap().cancel = Cancel::Move;
        send_mouse_button(game, id, MouseButton::Left, true);
        take(state);
        move_mouse(game, id, 312.0, 123.0);
        assert_eq!(take(state), [Seen::Moved([2.0, 3.0], [312.0, 123.0])]);
        assert_eq!(game.cursor(), [312.0, 123.0]);
        send_mouse_button(game, id, MouseButton::Left, false);
        state.lock().unwrap().cancel = Cancel::None;
        take(state);

        // The pad's pointer is not the mouse.
        game.client.pad.0.menu.synthetic = true;
        move_mouse(game, id, 330.0, 130.0);
        game.client.pad.0.menu.synthetic = false;
        assert!(take(state).is_empty());

        // The grab: raw motion goes to the screen, not the camera or the moved hook.
        assert!(game.test_set_pointer_grab(true));
        assert!(game.test_pointer_grabbed(), "reported at once");
        game.test_apply_pointer_grab_request();
        assert!(
            game.client.captured
                && game.test_mod_screen_grabs_pointer()
                && game.test_pointer_grabbed()
        );
        let turn = game.test_pointer_accumulated();
        game.test_grabbed_pointer_motion([3.5, -2.0]);
        game.test_grabbed_pointer_motion([0.0, 0.0]);
        assert_eq!(
            take(state),
            [Seen::Motion("input_route".into(), [3.5, -2.0])]
        );
        assert_eq!(
            game.test_pointer_accumulated(),
            turn,
            "the camera does not turn"
        );
        let held = game.cursor();
        move_mouse(game, id, 340.0, 140.0);
        send_mouse_button(game, id, MouseButton::Left, true);
        take(state);
        move_mouse(game, id, 350.0, 150.0);
        send_mouse_button(game, id, MouseButton::Left, false);
        assert!(
            !take(state)
                .iter()
                .any(|seen| matches!(seen, Seen::Moved(..) | Seen::DragPre(..))),
            "the grab's cursor moves are neither hover nor drag"
        );
        assert_eq!(game.cursor(), held);
        // Focus loss releases it, and refocus does not take it back on a screen.
        send(game, id, WindowEvent::Focused(false));
        assert!(!game.client.captured && !game.test_screen_grab() && !game.test_pointer_grabbed());
        send(game, id, WindowEvent::Focused(true));
        assert!(!game.client.captured && !game.test_pointer_grabbed());
        // The mod's own call, both ways.
        assert!(game.test_set_pointer_grab(true));
        game.test_apply_pointer_grab_request();
        assert!(game.test_pointer_grabbed());
        assert!(game.test_set_pointer_grab(false));
        assert!(!game.test_pointer_grabbed(), "reported at once");
        game.test_apply_pointer_grab_request();
        assert!(!game.client.captured && !game.test_screen_grab());
        game.test_screen_pointer_motion([1.0, 1.0]);
        assert!(take(state).is_empty(), "motion is the grab's");
        // Switching the mod off releases it at the next event.
        assert!(game.test_set_pointer_grab(true));
        game.test_apply_pointer_grab_request();
        assert!(game.test_screen_grab());
        assert!(mod_api::__private::set_mod_enabled("input_route", false));
        game.test_apply_pointer_grab_request();
        assert!(!game.client.captured && !game.test_screen_grab());
        assert!(mod_api::__private::set_mod_enabled("input_route", true));
        // Closing the screen releases it.
        assert!(game.test_set_pointer_grab(true));
        game.test_apply_pointer_grab_request();
        game.mod_screen_close("input_route");
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(!game.test_screen_grab() && !game.test_pointer_grabbed());
        assert!(!game.test_set_pointer_grab(true), "no screen is open");
    });
}

#[cfg(any(unix, windows))]
#[test]
#[ignore = "daily"]
fn dropped_files_reach_a_mod_screen_whole_and_in_order() {
    use std::path::PathBuf;
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::dropped_files_reach_a_mod_screen_whole_and_in_order"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_FILES_DROPPED_CHILD",
        &["drop"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, state| {
        let first = PathBuf::from("/tmp/a.png");
        let second = not_utf8_path();
        // No screen open: the drop fires nothing.
        send(game, id, WindowEvent::DroppedFile(first.clone()));
        game.idle(&NoLoop);
        assert!(take(state).is_empty());

        assert!(game.mod_screen_open("input_route"));
        send(game, id, WindowEvent::DroppedFile(first.clone()));
        send(game, id, WindowEvent::DroppedFile(second.clone()));
        assert!(
            take(state).is_empty(),
            "the list is fired when the pass ends"
        );
        game.idle(&NoLoop);
        let seen = take(state);
        assert_eq!(seen, [Seen::Dropped("mod".into(), vec![first, second])]);
        let Seen::Dropped(_, paths) = &seen[0] else {
            unreachable!()
        };
        assert_eq!(
            paths[1].to_str(),
            None,
            "the second path is still not UTF-8"
        );
        assert!(
            game.mod_screen_open("input_route"),
            "observe only: the screen stays"
        );
        assert_eq!(game.mod_screen(), Some("input_route"));
        game.idle(&NoLoop);
        assert!(take(state).is_empty(), "a drop fires once");
    });
}

#[cfg(any(unix, windows))]
fn not_utf8_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(b"/tmp/a\xff.png").to_owned()
    };
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<u16> = "/tmp/a".encode_utf16().collect();
        units.push(0xd800);
        units.extend(".png".encode_utf16());
        std::ffi::OsString::from_wide(&units)
    };
    path.into()
}

#[test]
#[ignore = "daily"]
fn a_drop_reaches_its_screen_before_a_click_that_closes_it() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_drop_reaches_its_screen_before_a_click_that_closes_it"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_FILES_DROPPED_CLOSE_CHILD",
        &["drop"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, state| {
        game.client.pause_menu = true;
        game.set_screen(Screen::Paused);
        publish_frame(game, 1);
        let (w, h) = game.gui_size();
        let (x, y, _, _, _, _) = game.pause_layout(w, h).buttons[0];
        move_mouse(game, id, f64::from(x + 10.0), f64::from(y + 10.0));
        take(state);
        let files = vec![
            std::path::PathBuf::from("pack.zip"),
            std::path::PathBuf::from("shot.png"),
        ];
        for file in &files {
            send(game, id, WindowEvent::DroppedFile(file.clone()));
        }
        // Back to Game: the click closes the pause menu in this pass.
        send_mouse_button(game, id, MouseButton::Left, true);
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(
            take(state),
            [
                Seen::Dropped("paused".into(), files),
                Seen::PressPre,
                Seen::PressPost("paused".into(), true)
            ]
        );
        game.idle(&NoLoop);
        assert!(take(state).is_empty(), "a drop fires once");
    });
}

#[test]
#[ignore = "daily"]
fn a_pointer_grab_is_refused_on_another_mods_screen() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_pointer_grab_is_refused_on_another_mods_screen"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_GRAB_OTHER_MOD_CHILD",
        &["grab"],
    ) {
        return;
    }
    with_input_game_as("someone_else", |game, _, _| {
        assert!(game.mod_screen_open("input_route"));
        assert!(!game.test_set_pointer_grab(true));
        assert!(!game.test_pointer_grabbed());
        game.test_apply_pointer_grab_request();
        assert!(
            !game.test_screen_grab()
                && !game.client.captured
                && game.mod_screen() == Some("input_route")
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_pointer_grab_is_refused_with_no_screen_open() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_pointer_grab_is_refused_with_no_screen_open"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_GRAB_NO_SCREEN_CHILD",
        &["grab"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, _, _| {
        assert_eq!(game.mod_screen(), None);
        assert!(!game.test_set_pointer_grab(true));
        assert!(!game.test_pointer_grabbed());
        game.test_apply_pointer_grab_request();
        assert!(!game.test_screen_grab());
    });
}

#[test]
#[ignore = "daily"]
fn a_pointer_grab_is_refused_while_the_window_is_unfocused() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_pointer_grab_is_refused_while_the_window_is_unfocused"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_SCREEN_GRAB_UNFOCUSED_CHILD",
        &["grab"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, _| {
        assert!(game.mod_screen_open("input_route"));
        send(game, id, WindowEvent::Focused(false));
        assert!(!game.test_set_pointer_grab(true));
        assert!(!game.test_pointer_grabbed());
        game.test_apply_pointer_grab_request();
        assert!(!game.test_screen_grab() && !game.client.captured);
    });
}

#[test]
#[ignore = "daily"]
fn context_menu_overlay_draw_order_disposal_and_action_reopen() {
    use mod_api::client::widgets::ContextMenuEntry;
    use sunrise_client_gameplay::game::client_side::books::BookScreen;

    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::context_menu_overlay_draw_order_disposal_and_action_reopen",
        "SUNRISE_CONTEXT_MENU_LIFECYCLE_CHILD",
        &["lifecycle"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, _| {
        // Overlay ordering: context menu draws after book overlay using the batch's GUI size.
        game.open_book(BookScreen::View {
            pages: vec![sunrise_client::text::Text::literal("hello")],
            page: 0,
            lectern: None,
        });
        let book_only = game.hud([320, 200]);
        let book_verts = book_only.vertices.len();
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = DropCounter(drops.clone());
        assert!(ScreensClientHost::open_context_menu(
            game,
            [310.0, 190.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Act"),
                move |_| {
                    let _keep = &guard;
                }
            )],
        ));
        let with_menu = game.hud([320, 200]);
        assert!(
            with_menu.vertices.len() > book_verts,
            "context menu emits vertices after the book overlay"
        );
        // Verify relative draw order: identifiable book vertices come strictly before menu border.
        let last_book_idx = book_only
            .vertices
            .iter()
            .rposition(|v| {
                v.pos[0] >= 64.0 && v.pos[0] <= 256.0 && v.pos[1] >= 2.0 && v.pos[1] <= 194.0
            })
            .expect("book geometry present");
        assert!(
            with_menu.vertices[..book_verts]
                .iter()
                .zip(&book_only.vertices)
                .all(|(a, b)| a.pos == b.pos && a.color == b.color),
            "book geometry remains intact before the menu"
        );
        let menu_border = with_menu.vertices[book_verts].color;
        assert!(
            book_only.vertices.iter().all(|v| v.color != menu_border),
            "menu border color is distinct from book geometry"
        );
        let first_menu_idx = with_menu
            .vertices
            .iter()
            .position(|v| v.color == menu_border)
            .expect("menu border drawn");
        assert!(
            last_book_idx < first_menu_idx,
            "context menu draws after book geometry"
        );
        let menu_verts = &with_menu.vertices[first_menu_idx..];
        let box_verts: Vec<_> = menu_verts.iter().filter(|v| v.text == 0).collect();
        assert!(!box_verts.is_empty(), "menu box geometry present");
        assert!(
            box_verts.iter().all(|v| v.pos[0] >= 0.0
                && v.pos[0] <= 320.0
                && v.pos[1] >= 0.0
                && v.pos[1] <= 200.0),
            "menu box clamps to the render batch's 320x200 GUI size"
        );
        assert!(
            box_verts.iter().any(|v| v.pos[0] == 320.0)
                && box_verts.iter().any(|v| v.pos[1] == 200.0),
            "menu box clamps against the 320x200 batch edges"
        );

        // Scrollable menu in a 7-11px-high GUI draws its scrollbar without panicking.
        let tall: Vec<_> = (0..12)
            .map(|i| ContextMenuEntry::action(ApiText::literal(format!("Row {i}")), |_| {}))
            .collect();
        assert!(ScreensClientHost::open_context_menu(game, [0.0, 0.0], tall));
        for h in [7, 9, 11] {
            let tiny = game.hud([100, h]);
            assert!(!tiny.vertices.is_empty());
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        // Replacing an open menu drops the previous entries immediately.
        let drops2 = Arc::new(AtomicUsize::new(0));
        let guard2 = DropCounter(drops2.clone());
        assert!(ScreensClientHost::open_context_menu(
            game,
            [40.0, 30.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Two"),
                move |_| {
                    let _keep = &guard2;
                }
            )],
        ));
        assert_eq!(drops2.load(Ordering::SeqCst), 0);

        // Screen replacement clears and disposes the menu in bump_instance immediately.
        assert!(game.mod_screen_open("input_route"));
        assert_eq!(drops2.load(Ordering::SeqCst), 1);
        assert!(!game.test_context_menu_open());

        // Disabled owner's menu is omitted from drawing and removed during refresh_screen_widgets.
        let drops3 = Arc::new(AtomicUsize::new(0));
        let guard3 = DropCounter(drops3.clone());
        assert!(ScreensClientHost::open_context_menu(
            game,
            [40.0, 30.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Dis"),
                move |_| {
                    let _keep = &guard3;
                }
            )],
        ));
        let before_disable = game.hud([320, 200]).vertices.len();
        assert!(mod_api::__private::set_mod_enabled("input_route", false));
        let while_disabled = game.hud([320, 200]).vertices.len();
        assert!(while_disabled < before_disable);
        assert_eq!(drops3.load(Ordering::SeqCst), 1);
        assert!(mod_api::__private::set_mod_enabled("input_route", true));

        let drops4 = Arc::new(AtomicUsize::new(0));
        let guard4 = DropCounter(drops4.clone());
        assert!(ScreensClientHost::open_context_menu(
            game,
            [40.0, 30.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("DisMaint"),
                move |_| {
                    let _keep = &guard4;
                }
            )],
        ));
        assert!(mod_api::__private::set_mod_enabled("input_route", false));
        game.test_refresh_screen_widgets();
        assert_eq!(drops4.load(Ordering::SeqCst), 1);
        assert!(!game.test_has_context_menu());
        assert!(mod_api::__private::set_mod_enabled("input_route", true));

        // Selected action runs once with the menu already closed and can reopen a new menu.
        let runs = Arc::new(AtomicUsize::new(0));
        let closed_before_run = Arc::new(AtomicBool::new(false));
        let reopened = Arc::new(AtomicBool::new(false));
        let runs_c = runs.clone();
        let closed_c = closed_before_run.clone();
        let reopened_c = reopened.clone();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [40.0, 30.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Reopen"),
                move |host| {
                    runs_c.fetch_add(1, Ordering::SeqCst);
                    closed_c.store(!host.close_context_menu(), Ordering::SeqCst);
                    reopened_c.store(
                        host.open_context_menu(
                            [50.0, 40.0],
                            vec![ContextMenuEntry::action(ApiText::literal("Next"), |_| {})],
                        ),
                        Ordering::SeqCst,
                    );
                },
            )],
        ));
        move_mouse(game, id, 48.0, 36.0);
        send_mouse_button(game, id, MouseButton::Left, true);
        send_mouse_button(game, id, MouseButton::Left, false);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(closed_before_run.load(Ordering::SeqCst));
        assert!(reopened.load(Ordering::SeqCst));
        assert!(game.test_context_menu_open());
    });
}

#[test]
#[ignore = "daily"]
fn context_menu_blocks_pointer_motion_drag_wheel_keyboard_ime_and_socket() {
    use mod_api::client::widgets::ContextMenuEntry;
    use winit::keyboard::NativeKeyCode;

    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::context_menu_blocks_pointer_motion_drag_wheel_keyboard_ime_and_socket",
        "SUNRISE_CONTEXT_MENU_ROUTING_CHILD",
        &["routing"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, state| {
        state.lock().unwrap().record_pointer = true;
        assert!(game.mod_screen_open("input_route"));
        publish_frame(game, 1);

        // Start a screen + mounted-widget drag, then open the context menu: motion, drag, and
        // wheel stay out of the underlying screen and its mounted widgets while the menu owns the
        // pointer, and releasing the screen-owned press still cleans up drag_button and widget drag.
        move_mouse(game, id, 20.0, 95.0);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert!(game.client.drag_button.is_some());
        assert!(game.client.screen_state.0.widgets.dragging().is_some());
        take(state);
        assert!(ScreensClientHost::open_context_menu(
            game,
            [200.0, 100.0],
            vec![ContextMenuEntry::action(ApiText::literal("Pick"), |_| {})],
        ));
        move_mouse(game, id, 210.0, 110.0);
        assert_eq!(game.cursor(), [210.0, 110.0]);
        send(
            game,
            id,
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(0.0, -1.0),
                phase: TouchPhase::Moved,
            },
        );
        assert!(
            take(state).is_empty(),
            "no screen move, drag or wheel while menu is open"
        );
        // Releasing the screen-owned press cleans up drag_button and mounted-widget drag.
        send_mouse_button(game, id, MouseButton::Left, false);
        assert!(game.client.drag_button.is_none());
        assert!(game.client.screen_state.0.widgets.dragging().is_none());
        assert!(ScreensClientHost::close_context_menu(game));

        // Focus Chat with active IME preedit, then open menu: preedit resets, unidentified-key
        // characters (Alt-code), IME preedit/commit, and control socket keys never reach Chat.
        game.chat_ui_mut_for_test().chat.add_recent("older");
        game.begin_chat_screen("base", false);
        send(
            game,
            id,
            WindowEvent::Ime(Ime::Preedit("ka".into(), Some((0, 2)))),
        );
        assert!(preedit::composing());
        let picked = Arc::new(AtomicBool::new(false));
        let picked_c = picked.clone();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [60.0, 60.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Go"),
                move |_| {
                    picked_c.store(true, Ordering::SeqCst);
                }
            )],
        ));
        assert!(
            !preedit::composing(),
            "opening menu resets active IME composition"
        );
        // Unidentified-key text (Windows Alt-code) is consumed.
        window_key(
            game,
            WindowKeyInput {
                physical: PhysicalKey::Unidentified(NativeKeyCode::Unidentified),
                state: ElementState::Pressed,
                repeat: false,
                text: Some("é"),
                shortcut: None,
            },
        );
        // IME preedit and commit are suppressed while menu is open.
        send(
            game,
            id,
            WindowEvent::Ime(Ime::Preedit("ni".into(), Some((0, 2)))),
        );
        assert!(!preedit::composing());
        send(game, id, WindowEvent::Ime(Ime::Commit("日".into())));
        assert_eq!(
            game.chat_ui_for_test()
                .chat_screen
                .as_ref()
                .unwrap()
                .value(),
            "base"
        );

        // Actual control socket `key` command goes through the menu before Chat.
        send_key_control(game, "key down tap");
        send_key_control(game, "key enter tap");
        assert!(picked.load(Ordering::SeqCst));
        assert_eq!(game.client.screen, Screen::Chat);
        assert_eq!(
            game.chat_ui_for_test()
                .chat_screen
                .as_ref()
                .unwrap()
                .value(),
            "base"
        );

        // With the chat on-screen keyboard already open, right-stick input and OSK output
        // stay gated while the context menu is active and do not change chat history/suggestions
        // even on the first closed-menu poll after dismissing with a controller button.
        game.client.settings.controller.enabled = true;
        game.client.pad.0.test_set_in_use(true);
        game.client.pad.0.menu.osk = Some(sunrise_client::osk::Keyboard::new(false));
        let tall: Vec<_> = (0..12)
            .map(|i| ContextMenuEntry::action(ApiText::literal(format!("Row {i}")), |_| {}))
            .collect();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [20.0, 20.0],
            tall
        ));
        game.client.pad.0.state.right = [0.0, 1.0];
        game.test_pad_menu_frame(&NoLoop);
        game.pad_osk_output(sunrise_client::osk::Output::Previous);
        assert_eq!(
            game.chat_ui_for_test()
                .chat_screen
                .as_ref()
                .unwrap()
                .value(),
            "base"
        );
        // Dismiss via controller button while still holding the right stick up: the wheel
        // accumulator resets and the first closed-menu poll must not trigger OSK Previous.
        game.pad_menu_button(&NoLoop, sunrise_client::gamepad::Button::East, true)
            .unwrap();
        game.pad_menu_button(&NoLoop, sunrise_client::gamepad::Button::East, false)
            .unwrap();
        assert!(!game.test_context_menu_open());
        assert_eq!(game.client.pad.0.menu.test_wheel(), [0.0, 0.0]);
        game.test_pad_menu_frame(&NoLoop);
        assert_eq!(
            game.chat_ui_for_test()
                .chat_screen
                .as_ref()
                .unwrap()
                .value(),
            "base",
            "held right stick stays suppressed until neutral after menu dismissal"
        );
        game.client.pad.0.state.right = [0.0, 0.0];
        game.test_pad_menu_frame(&NoLoop);
        game.client.pad.0.state.right = [0.0, 1.0];
        game.test_pad_menu_frame(&NoLoop);
        assert_eq!(
            game.chat_ui_for_test()
                .chat_screen
                .as_ref()
                .unwrap()
                .value(),
            "older"
        );
        game.client.pad.0.state.right = [0.0, 0.0];
        game.client.pad.0.menu.osk = None;

        // Releasing an F3+C crash chord while a context menu is open still cancels the
        // countdown and clears modifier_down without firing debug overlay or screen actions.
        game.test_close_chat_done();
        game.client.debug = false;
        send_key_control(game, "key f3 down");
        send_key_control(game, "key c down");
        assert!(game.client.test_debug_keys().modifier_down);
        assert!(game.client.test_debug_keys().crash_key_time > 0);
        game.test_open_gamemode_switcher();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [60.0, 60.0],
            vec![ContextMenuEntry::action(ApiText::literal("Keep"), |_| {})],
        ));
        send_key_control(game, "key c up");
        send_key_control(game, "key f3 up");
        assert_eq!(game.client.test_debug_keys().crash_key_time, -1);
        assert!(!game.client.test_debug_keys().modifier_down);
        assert!(!game.client.debug);
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);
        assert!(game.test_context_menu_open());
        let future = game.test_debug_millis() + 20_000;
        assert_eq!(game.client.test_tick_debug_keys(future), None);
        assert!(ScreensClientHost::close_context_menu(game));
        game.test_close_gamemode_switcher(false);

        // Over GameModeSwitcher: Escape, arrows and Enter route to the menu ahead of the
        // switcher's debug_key handler in both window and control-socket `key` entry points.
        game.test_open_gamemode_switcher();
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);
        let sw_picked = Arc::new(AtomicBool::new(false));
        let sw_picked_c = sw_picked.clone();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [60.0, 60.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Pick"),
                move |_| {
                    sw_picked_c.store(true, Ordering::SeqCst);
                }
            )],
        ));
        assert!(!game.debug_key(
            KeyCode::Escape,
            sunrise_client::debug_keys::KeyAction::Press
        ));
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);
        send_key_control(game, "key down tap");
        send_key_control(game, "key enter tap");
        assert!(sw_picked.load(Ordering::SeqCst));
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);

        assert!(ScreensClientHost::open_context_menu(
            game,
            [60.0, 60.0],
            vec![ContextMenuEntry::action(ApiText::literal("Stay"), |_| {})],
        ));
        send_key_control(game, "key escape down");
        assert!(!game.test_context_menu_open());
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);
        send_key_control(game, "key escape repeat");
        send_key_control(game, "key escape up");
        assert_eq!(game.client.screen, Screen::GameModeSwitcher);
        send_key_control(game, "key escape tap");
        assert_eq!(game.client.screen, Screen::Playing);
    });
}

#[test]
#[ignore = "daily"]
fn context_menu_tracks_mouse_key_and_controller_press_ownership() {
    use mod_api::client::widgets::ContextMenuEntry;
    use sunrise_client::gamepad::Button as PadButton;

    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::context_menu_tracks_mouse_key_and_controller_press_ownership",
        "SUNRISE_CONTEXT_MENU_OWNERSHIP_CHILD",
        &["ownership"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, id, state| {
        assert!(game.mod_screen_open("input_route"));
        publish_frame(game, 1);

        // Outside press closes the menu on mouse-down and swallows the matching mouse-up;
        // a controller-generated (Game origin) release cannot steal User press ownership.
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(ApiText::literal("Item"), |_| {})],
        ));
        move_mouse(game, id, 15.0, 25.0);
        take(state);
        send_mouse_button(game, id, MouseButton::Left, true);
        assert!(!game.test_context_menu_open());
        game.handle_window_event(
            &NoLoop,
            id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: ElementState::Released,
                button: MouseButton::Left,
            },
            InputOrigin::Game,
        );
        take(state);
        send_mouse_button(game, id, MouseButton::Left, false);
        assert!(
            take(state).is_empty(),
            "User mouse-up after menu-owned dismissal is still swallowed"
        );

        // Holding Escape after dismissing the menu swallows repeats and release so the screen
        // underneath stays open, even if a socket edge for Escape arrives in between.
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(ApiText::literal("Item"), |_| {})],
        ));
        key(game, KeyCode::Escape, ElementState::Pressed);
        assert!(!game.test_context_menu_open());
        assert_eq!(game.client.screen, Screen::Mod);
        send_key_control(game, "key escape up");
        assert_eq!(game.client.screen, Screen::Mod);
        window_key(
            game,
            WindowKeyInput {
                physical: PhysicalKey::Code(KeyCode::Escape),
                state: ElementState::Pressed,
                repeat: true,
                text: None,
                shortcut: None,
            },
        );
        assert_eq!(
            game.client.screen,
            Screen::Mod,
            "Escape repeat must not close screen"
        );
        key(game, KeyCode::Escape, ElementState::Released);
        assert_eq!(game.client.screen, Screen::Mod);

        // Controller focus mode with an armed mounted widget underneath: D-pad + South selects
        // menu action without activating the armed widget, and East/Start dismisses without
        // closing the screen or firing the armed widget.
        let targets = game.mounted_widget_targets();
        let mounted_button = *targets.last().expect("mounted button target");
        game.client.pad.0.menu.focus = Some(mounted_button);
        *game.client.pad.0.menu.test_armed_mut() = Some(mounted_button);
        take(state);
        let ran = Arc::new(AtomicBool::new(false));
        let ran_c = ran.clone();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("PadAct"),
                move |_| {
                    ran_c.store(true, Ordering::SeqCst);
                }
            )],
        ));
        assert!(game.client.pad.0.menu.test_armed_mut().is_none());
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, true)
            .unwrap();
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, false)
            .unwrap();
        game.pad_menu_button(&NoLoop, PadButton::South, true)
            .unwrap();
        game.pad_menu_button(&NoLoop, PadButton::South, false)
            .unwrap();
        assert!(ran.load(Ordering::SeqCst));
        assert!(!game.test_context_menu_open());
        assert!(
            !take(state).iter().any(|s| matches!(s, Seen::Activation(_))),
            "armed mounted widget underneath must not activate"
        );

        // Dismissing the menu via keyboard while a controller direction is held clears the
        // menu's pending repeat and suppresses the held stick until neutral.
        game.client.settings.controller.enabled = true;
        game.client.pad.0.test_set_in_use(true);
        game.client.pad.0.menu.focus = Some(mounted_button);
        game.client
            .pad
            .0
            .menu
            .test_set_pos(Some(mounted_button.center()));
        game.client.cursor = mounted_button.center();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("HoldDir"),
                |_| {}
            )],
        ));
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, true)
            .unwrap();
        key(game, KeyCode::Escape, ElementState::Pressed);
        key(game, KeyCode::Escape, ElementState::Released);
        assert!(!game.test_context_menu_open());
        game.test_pad_menu_frame(&NoLoop);
        assert_eq!(game.client.pad.0.menu.focus, Some(mounted_button));
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, false)
            .unwrap();

        // Socket `release` while holding D-pad Down in the menu keeps controller ownership so
        // dismissing via socket still clears `pad.menu.repeat` and never steps the screen underneath.
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("SocketRel"),
                |_| {}
            )],
        ));
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, true)
            .unwrap();
        send_key_control(game, "release");
        send_key_control(game, "key escape tap");
        assert!(!game.test_context_menu_open());
        assert!(
            game.client.pad.0.menu.test_repeat().is_none(),
            "controller repeat must not remain armed after socket release + socket dismiss"
        );
        game.test_pad_menu_frame(&NoLoop);
        assert_eq!(game.client.pad.0.menu.focus, Some(mounted_button));
        game.pad_menu_button(&NoLoop, PadButton::DPadDown, false)
            .unwrap();

        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("StickDir"),
                |_| {}
            )],
        ));
        game.client.pad.0.state.left = [0.0, -1.0];
        game.test_pad_menu_frame(&NoLoop);
        assert!(ScreensClientHost::close_context_menu(game));
        game.test_pad_menu_frame(&NoLoop);
        assert_eq!(
            game.client.pad.0.menu.focus,
            Some(mounted_button),
            "held stick stays suppressed until neutral after dismissal"
        );
        game.client.pad.0.state.left = [0.0, 0.0];
        game.test_pad_menu_frame(&NoLoop);

        // East/Start dismisses menu without closing the mod screen underneath.
        game.client.pad.0.menu.focus = Some(mounted_button);
        *game.client.pad.0.menu.test_armed_mut() = Some(mounted_button);
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("CloseMe"),
                |_| {}
            )],
        ));
        game.pad_menu_button(&NoLoop, PadButton::East, true)
            .unwrap();
        game.pad_menu_button(&NoLoop, PadButton::East, false)
            .unwrap();
        assert!(!game.test_context_menu_open());
        assert_eq!(game.client.screen, Screen::Mod);

        // Space-selected menu action that closes the mod screen and recaptures the pointer
        // must not restore Space as held Jump while Space remains down.
        let jump = VanillaMappingSlot::by_default_key(KeyCode::Space).unwrap();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("CloseScreen"),
                |host| {
                    host.close_screen();
                }
            )],
        ));
        key(game, KeyCode::ArrowDown, ElementState::Pressed);
        key(game, KeyCode::ArrowDown, ElementState::Released);
        game.client.physical_keys.insert(KeyCode::Space);
        key(game, KeyCode::Space, ElementState::Pressed);
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(game.client.captured);
        assert!(
            !game.client.vanilla_mappings.is_down(jump) && !game.client.keys.raw(&KeyCode::Space),
            "menu-owned Space must not be restored as held Jump on pointer recapture"
        );
        game.client.physical_keys.remove(&KeyCode::Space);
        key(game, KeyCode::Space, ElementState::Released);
        assert!(
            !game.client.vanilla_mappings.is_down(jump) && !game.client.keys.raw(&KeyCode::Space)
        );

        // A socket-owned Space press in the menu must not suppress a fresh physical Keyboard
        // Space press during pointer recapture.
        assert!(game.mod_screen_open("input_route"));
        assert!(ScreensClientHost::open_context_menu(
            game,
            [100.0, 100.0],
            vec![ContextMenuEntry::action(ApiText::literal("Hold"), |_| {})],
        ));
        send_key_control(game, "key space down");
        assert!(ScreensClientHost::close_context_menu(game));
        game.client.physical_keys.insert(KeyCode::Space);
        key(game, KeyCode::Space, ElementState::Pressed);
        game.close_mod_screen();
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(game.client.captured);
        assert!(
            game.client.vanilla_mappings.is_down(jump)
                && game
                    .client
                    .keys
                    .held_by(&KeyCode::Space, sunrise_client::held::Source::Keyboard),
            "fresh physical Space must still restore Jump despite socket-owned Space"
        );
        send_key_control(game, "key space up");
        game.client.physical_keys.remove(&KeyCode::Space);
        key(game, KeyCode::Space, ElementState::Released);
    });
}

#[test]
#[ignore = "daily"]
fn context_menu_and_pointer_grab_refuse_each_other_in_both_orders() {
    use mod_api::client::widgets::ContextMenuEntry;

    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::input_routing::context_menu_and_pointer_grab_refuse_each_other_in_both_orders",
        "SUNRISE_CONTEXT_MENU_GRAB_CHILD",
        &["grab"],
    ) {
        return;
    }
    with_input_game_as("input_route", |game, _, _| {
        assert!(game.mod_screen_open("input_route"));

        // Order 1: pending or active pointer grab refuses context menu and disposes its entries.
        assert!(game.test_set_pointer_grab(true));
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = DropCounter(drops.clone());
        assert!(!ScreensClientHost::open_context_menu(
            game,
            [40.0, 40.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Blocked"),
                move |_| {
                    let _keep = &guard;
                }
            )],
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        game.test_apply_pointer_grab_request();
        assert!(game.test_mod_screen_grabs_pointer());
        assert!(!ScreensClientHost::open_context_menu(
            game,
            [40.0, 40.0],
            vec![ContextMenuEntry::action(
                ApiText::literal("Blocked2"),
                |_| {}
            )],
        ));

        // Releasing the grab allows opening the menu again.
        assert!(game.test_set_pointer_grab(false));
        game.test_apply_pointer_grab_request();
        assert!(ScreensClientHost::open_context_menu(
            game,
            [40.0, 40.0],
            vec![ContextMenuEntry::action(ApiText::literal("Open"), |_| {})],
        ));

        // Order 2: an open context menu refuses new pointer grab requests until it closes.
        assert!(!game.test_set_pointer_grab(true));
        assert!(!game.test_pointer_grabbed());
        game.test_apply_pointer_grab_request();
        assert!(!game.test_screen_grab() && !game.client.captured);

        assert!(ScreensClientHost::close_context_menu(game));
        assert!(game.test_set_pointer_grab(true));
        game.test_apply_pointer_grab_request();
        assert!(game.test_mod_screen_grabs_pointer());
    });
}
