#![cfg(test)]
use sunrise_game_data::text::Text;
#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenOpening, ScreenOpeningEvent};
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_client::chat::{ChatAbilities, ChatVisibility};
    use sunrise_gameplay_common::game::Screen;
    use sunrise_narrator::{GameNarrator, Sink, Status};
    use sunrise_protocol::packets::play::{Clientbound, GameEvent, PlayerPosition};
    use {
        crate::game::remote::tests::connect_menu_test, crate::game::remote::tests::spawn_menu_test,
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::in_own_process,
    };

    struct Narration(Arc<Mutex<Vec<(String, bool)>>>);
    impl Sink for Narration {
        fn say(&mut self, message: &str, interrupt: bool, _: f32) {
            self.0.lock().unwrap().push((message.to_owned(), interrupt));
        }
        fn clear(&mut self) {}
        fn active(&mut self) -> bool {
            true
        }
    }

    struct KeepDemoOpen(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for KeepDemoOpen {
        fn screen_opening(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            (event.current.id == "demo_intro"
                && event.new.id == "playing"
                && self.0.load(Ordering::Relaxed))
            .then_some(ScreenOpening::Cancel)
        }
    }

    #[test]
    #[ignore = "daily"]
    fn demo_packets_open_the_parentless_intro_and_narrate_only_exact_hints() {
        in_own_process(
            "cut_layout::game::client_side::demo_intro::tests::demo_packets_open_the_parentless_intro_and_narrate_only_exact_hints",
            || {
                let veto = Arc::new(AtomicBool::new(false));
                let mods = vec![ModEntry::new(
                    "demo_observer",
                    Box::new(KeepDemoOpen(veto.clone())),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_client_game(|game| {
                        let (_reader, _writer) = connect_menu_test(game);
                        spawn_menu_test(game);
                        game.handle_packet(Clientbound::PlayerPosition(PlayerPosition {
                            teleport: 1,
                            position: [0.5, 65.0, 0.5],
                            delta: [0.0; 3],
                            yaw: 0.0,
                            pitch: 0.0,
                            relatives: 0,
                        }))
                        .unwrap();
                        game.client.remote.as_mut().unwrap().level_load.finish();
                        game.client.focused = true;
                        game.client.chat_ui.chat.set_abilities(
                            &game.client.gui.atlas.font,
                            ChatAbilities::new(ChatVisibility::Hidden),
                        );
                        let spoken = Arc::new(Mutex::new(Vec::new()));
                        sunrise_narrator::with_narrator(|narrator| {
                            *narrator =
                                GameNarrator::with_sink(Box::new(Narration(spoken.clone())));
                            narrator.set_status(Status::System);
                        });
                        for (mapping, key) in [
                            ("key.forward", "key.keyboard.i"),
                            ("key.left", "key.keyboard.j"),
                            ("key.back", "key.keyboard.k"),
                            ("key.right", "key.keyboard.l"),
                            ("key.jump", "key.keyboard.q"),
                            ("key.inventory", "key.keyboard.r"),
                        ] {
                            game.client.settings.keys.set(mapping, key);
                        }
                        let send = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                    param| {
                            game.handle_packet(Clientbound::GameEvent(GameEvent {
                                event: 5,
                                param,
                            }))
                            .unwrap()
                        };
                        for zero in [0.0, -0.0] {
                            game.set_screen(Screen::Paused);
                            let count = game.client.remote.as_ref().unwrap().chat_received;
                            send(game, zero);
                            assert_eq!(game.client.screen, Screen::DemoIntro);
                            let intro = game.client.demo_intro.as_ref().unwrap();
                            assert_eq!(intro.title.translation_key(), Some("demo.help.title"));
                            let controls = intro.messages[0].component();
                            assert_eq!(
                                controls["extra"],
                                json!([
                                    {"translate": "demo.help.movementShort", "with": [{"text": "I"}, {"text": "J"}, {"text": "K"}, {"text": "L"}]},
                                    {"text": "\n"},
                                    {"translate": "demo.help.movementMouse"},
                                    {"text": "\n"},
                                    {"translate": "demo.help.jump", "with": [{"text": "Q"}]},
                                    {"text": "\n"},
                                    {"translate": "demo.help.inventory", "with": [{"text": "R"}]},
                                ])
                            );
                            assert_eq!(
                                intro.messages[1].translation_key(),
                                Some("demo.help.fullWrapped")
                            );
                            assert_eq!(
                                intro.buttons.each_ref().map(Text::translation_key),
                                [Some("demo.help.buy"), Some("demo.help.later")]
                            );
                            // Buy opens the trusted link confirmation on the HUD; closing
                            // it is `Gui.setScreen(null)` there, which grabs the mouse.
                            let (w, h) = game.gui_size();
                            let layout = intro.layout(&game.client.gui.atlas, w, h);
                            game.client.cursor = (layout.button_x[0] + 1.0, layout.button_y + 1.0);
                            game.client.agent_pointer = true;
                            assert!(game.demo_intro_click());
                            assert_eq!(game.client.screen, Screen::ConfirmLink);
                            game.close_confirm_link();
                            assert_eq!(
                                game.client.screen,
                                Screen::Playing,
                                "Buy returns to the HUD"
                            );
                            assert!(
                                !game.client.agent_pointer,
                                "closing the link confirmation requests capture"
                            );

                            game.set_screen(Screen::Paused);
                            send(game, zero);
                            assert_eq!(game.client.screen, Screen::DemoIntro);
                            let (w, h) = game.gui_size();
                            let layout = game.client.demo_intro.as_ref().unwrap().layout(
                                &game.client.gui.atlas,
                                w,
                                h,
                            );
                            game.client.cursor = (layout.button_x[1] + 1.0, layout.button_y + 1.0);
                            game.client.agent_pointer = true;
                            veto.store(true, Ordering::Relaxed);
                            assert!(game.demo_intro_click());
                            assert_eq!(game.client.screen, Screen::DemoIntro);
                            assert!(
                                game.client.agent_pointer,
                                "a veto leaves the popup and pointer alone"
                            );
                            veto.store(false, Ordering::Relaxed);
                            assert!(game.demo_intro_click());
                            assert_eq!(
                                game.client.screen,
                                Screen::Playing,
                                "Later does not return to Pause"
                            );
                            assert!(!game.client.agent_pointer, "Later requests capture");
                            assert_eq!(game.client.remote.as_ref().unwrap().chat_received, count);
                            assert!(spoken.lock().unwrap().is_empty());
                        }
                        for (phase, key, args) in [
                            (101.0, "demo.help.movement", &['I', 'J', 'K', 'L'][..]),
                            (102.0, "demo.help.jump", &['Q'][..]),
                            (103.0, "demo.help.inventory", &['R'][..]),
                            (104.0, "demo.day.6", &['Z'][..]),
                        ] {
                            if phase == 104.0 {
                                game.client
                                    .settings
                                    .keys
                                    .set("key.screenshot", "key.keyboard.z");
                            }
                            let count = game.client.remote.as_ref().unwrap().chat_received;
                            send(game, phase);
                            assert_eq!(game.client.screen, Screen::Playing);
                            assert_eq!(
                                game.client.remote.as_ref().unwrap().chat_received,
                                count + 1
                            );
                            let expected = json!({"translate": key, "with": args.iter().map(|label| json!({"text": label.to_string()})).collect::<Vec<_>>()});
                            assert_eq!(game.client.chat_ui.chat.recent_components(1), [expected]);
                            let chat = game.client.chat_ui.chat.recent_lines(1).pop().unwrap();
                            assert_eq!(
                                std::mem::take(&mut *spoken.lock().unwrap()),
                                [(chat, false)]
                            );
                        }
                        let count = game.client.remote.as_ref().unwrap().chat_received;
                        for phase in [
                            0.25,
                            -0.25,
                            100.75,
                            101.0_f32.next_up(),
                            102.0_f32.next_down(),
                            103.25,
                            104.25,
                            105.0,
                            f32::NAN,
                        ] {
                            send(game, phase);
                            assert_eq!(game.client.screen, Screen::Playing);
                            assert_eq!(game.client.remote.as_ref().unwrap().chat_received, count);
                            assert!(spoken.lock().unwrap().is_empty());
                        }
                        sunrise_narrator::set_status(Status::Chat);
                        send(game, 102.0);
                        assert_eq!(
                            game.client.remote.as_ref().unwrap().chat_received,
                            count + 1
                        );
                        assert!(
                            spoken.lock().unwrap().is_empty(),
                            "a demo hint is system narration, not chat narration"
                        );
                    });
                });
            },
        );
    }
}
