use crate::game::remote::claimed_item_use_tests::CUSTOM_DATA;
use crate::game::remote::claimed_item_use_tests::Call;
use crate::game::remote::claimed_item_use_tests::OWNER;
use crate::game::remote::claimed_item_use_tests::Phase;
use crate::game::remote::claimed_item_use_tests::SHIELD;
use crate::game::remote::claimed_item_use_tests::STICK;
use crate::game::remote::claimed_item_use_tests::with_owner;
use std::sync::Arc;
use sunrise_client::interaction_input::Button;
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_items::items::ItemStack;
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::Screen,
};
use {
    sunrise_core::geometry::Vec3, sunrise_protocol::packets::play,
    sunrise_protocol::packets::play::Clientbound as C, sunrise_sim::motion::PlayerMotion,
};

#[test]
#[ignore = "daily"]
fn owner_gets_release_or_reset_with_live_typed_stack() {
    fn marked(item: &str, count: u32, marker: Tag) -> ItemStack {
        let mut stack = ItemStack::new(item, count);
        stack
            .data_mut()
            .set_other(CUSTOM_DATA, Some(Tag::compound([("marker", marker)])));
        stack
    }
    fn start(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
        game.select_hotbar_slot(3).unwrap();
        game.inventory[3] = Some(marked(SHIELD, 1, Tag::Byte(23)));
        game.set_mapping_held("key.use", true);
        game.queue_mapping_click(Button::Use.mapping());
        game.remote_handle_input();
        assert!(game.client.client_item_use.0.is_some());
        assert!(game.using.is_none());
    }
    fn key_up(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
        game.set_mapping_held("key.use", false);
        game.remote_handle_input();
        assert!(game.client.client_item_use.0.is_none());
    }

    with_owner(|calls| {
        with_client_game(|game| {
            let _peer = logged_in(game);
            game.client.screen = Screen::Playing;
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            let shield = mod_api::__private::item_key(SHIELD).unwrap();
            let call = |phase, count, marker| Call {
                phase,
                registered_for: SHIELD,
                item: shield,
                slot: 3,
                count,
                marker: Some(marker),
            };
            let original = |phase| call(phase, 1, Tag::Byte(23));

            // Count and component mutations in the active slot are rebound on tick and release.
            start(game);
            assert_eq!(calls.take(), [original(Phase::Start)]);
            game.remote_tick_using();
            game.inventory[3] = Some(marked(SHIELD, 3, Tag::Int(99)));
            game.remote_tick_using();
            game.inventory[3] = Some(marked(SHIELD, 2, Tag::Int(101)));
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [
                    call(Phase::Release(2), 2, Tag::Int(101)),
                    call(Phase::Stop(2), 2, Tag::Int(101)),
                ]
            );

            // Another registered item (even for the same mod) cannot receive this use's end;
            // on_stop_using keeps the last valid stack of the captured item kind.
            start(game);
            game.inventory[3] = Some(marked(SHIELD, 4, Tag::Int(77)));
            game.remote_tick_using();
            game.inventory[3] = Some(marked(STICK, 1, Tag::Int(99)));
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [
                    original(Phase::Start),
                    call(Phase::Stop(1), 4, Tag::Int(77)),
                ]
            );

            // Emptying the active slot also preserves the last valid stack for on_stop_using.
            start(game);
            game.inventory[3] = Some(marked(SHIELD, 5, Tag::Int(88)));
            game.remote_tick_using();
            game.inventory[3] = None;
            game.remote_tick_using();
            assert!(game.client.client_item_use.0.is_none());
            assert_eq!(
                calls.take(),
                [
                    original(Phase::Start),
                    call(Phase::Stop(1), 5, Tag::Int(88)),
                ]
            );

            // The same kind in a different slot is a reset, even on the key-up tick.
            start(game);
            game.inventory[4] = Some(marked(SHIELD, 1, Tag::Int(99)));
            game.select_hotbar_slot(4).unwrap();
            key_up(game);
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(0))]
            );

            start(game);
            assert!(mod_api::__private::set_mod_enabled(OWNER, false));
            key_up(game);
            assert_eq!(calls.take(), [original(Phase::Start)]);
            assert!(mod_api::__private::set_mod_enabled(OWNER, true));

            // A keybind opening a screen releases mappings and runs release_using then on_stop_using.
            start(game);
            game.queue_mapping_click("key.chat");
            assert!(game.client.client_item_use.0.is_some());
            game.remote_handle_input();
            assert!(game.client.client_item_use.0.is_none());
            game.test_close_chat_intentionally();
            key_up(game);
            assert_eq!(
                calls.take(),
                [
                    original(Phase::Start),
                    original(Phase::Release(0)),
                    original(Phase::Stop(0)),
                ]
            );

            // Quick Actions (KeyG) with no dialog does not release an active use; when a
            // Quick Actions dialog opens, release_using then on_stop_using run on the same tick.
            start(game);
            game.key_pressed(
                winit::keyboard::KeyCode::KeyG,
                winit::keyboard::KeyCode::KeyG,
            )
            .unwrap();
            game.remote_handle_input();
            assert!(!game.dialog_open());
            assert!(game.client.client_item_use.0.is_some());
            {
                let registries =
                    Arc::make_mut(&mut game.client.remote.as_mut().unwrap().registries);
                let mut notice = std::collections::BTreeMap::new();
                notice.insert(
                    JavaString::from("type"),
                    Tag::String(JavaString::from("minecraft:notice")),
                );
                notice.insert(
                    JavaString::from("title"),
                    Tag::String(JavaString::from("Quick")),
                );
                registries.registries.insert(
                    "minecraft:dialog".into(),
                    vec![(
                        "test:quick".into(),
                        Some(Tag::Compound(notice.into_iter().collect())),
                    )],
                );
                registries
                    .tags
                    .entry("minecraft:dialog".into())
                    .or_default()
                    .insert("minecraft:quick_actions".into(), vec![0]);
            }
            game.key_pressed(
                winit::keyboard::KeyCode::KeyG,
                winit::keyboard::KeyCode::KeyG,
            )
            .unwrap();
            game.remote_handle_input();
            assert!(game.dialog_open());
            assert!(game.client.client_item_use.0.is_none());
            game.close_dialogs();
            key_up(game);
            assert_eq!(
                calls.take(),
                [
                    original(Phase::Start),
                    original(Phase::Release(0)),
                    original(Phase::Stop(0)),
                ]
            );

            // The death packet retires the capture without waiting for another input tick.
            start(game);
            game.handle_packet(C::SetHealth(play::SetHealth {
                health: 0.0,
                food: 20,
                saturation: 5.0,
            }))
            .unwrap();
            assert!(game.client.client_item_use.0.is_none());
            game.reset_client_item_use();
            assert_eq!(
                calls.take(),
                [original(Phase::Start), original(Phase::Stop(0))]
            );
        });

        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 65.0, 0.5)).unwrap());
            game.common.inventory[game.common.selected] = Some(marked(SHIELD, 1, Tag::Byte(23)));
            assert!(game.use_hand(None).unwrap());
            assert_eq!(game.using, Some((game.selected, 0)));
            assert!(
                calls.take().is_empty(),
                "normal server item use has no client owner"
            );
        });
    });
}
