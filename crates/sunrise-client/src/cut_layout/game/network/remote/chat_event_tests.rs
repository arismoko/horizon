use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::Flow;
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};

struct Censor;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Censor {
    fn client_chat_received(
        &self,
        _: &mut dyn mod_api::ClientHost,
        event: &mut mod_api::ClientChatReceivedEvent<'_>,
    ) -> Flow {
        use mod_api::text::TextContent;
        let text = match &event.message.content {
            TextContent::Literal(text) => text,
            // Player chat arrives decorated with the sender and message.
            TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" => {
                let TextContent::Literal(text) = &arguments[1].content else {
                    panic!("expected literal player message fixture");
                };
                text
            }
            _ => return Flow::Continue,
        };
        if text.contains("drop") {
            return Flow::Cancel;
        }
        // A system line carries no player message, so only it gets the tag.
        if event.system && event.player_message.is_none() {
            *event.message = mod_api::Text::literal(format!("[server] {text}"));
        }
        Flow::Continue
    }
}

/// `ClientChatReceivedEvent`: a mod's replacement is the line the chat
/// shows, a cancelled system line never shows, and a cancelled message
/// held by Chat Delay lets the next one through in the same tick.
#[test]
#[ignore = "daily"]
fn mods_replace_and_cancel_received_chat() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "test-chat-received",
            Box::new(Censor),
        )]
        .into(),
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ChatTest").unwrap();
                game.client.remote.as_mut().unwrap().has_logical_player = true;
                let lines = |game: &sunrise_client_gameplay::game::ClientGame<'_>| -> Vec<String> {
                    let chat = &game.client.remote.as_ref().unwrap().chat;
                    chat.iter().map(|(line, _)| line.clone()).collect()
                };
                let system = |content: &str| {
                    C::SystemChat(play::SystemChat {
                        content: Tag::String(content.into()),
                        overlay: false,
                    })
                };
                game.handle_packet(system("hello")).unwrap();
                game.handle_packet(system("drop me")).unwrap();
                assert_eq!(lines(game), ["[server] hello"]);

                game.client.settings.chat_delay = 1.0;
                let disguised = |message: &str| {
                    C::DisguisedChat(play::DisguisedChat {
                        message: Tag::String(message.into()),
                        chat_type: play::ChatTypeBound {
                            chat_type: 0,
                            name: Tag::String("Steve".into()),
                            target: None,
                        },
                    })
                };
                game.handle_packet(disguised("drop this")).unwrap();
                game.handle_packet(disguised("hi")).unwrap();
                assert_eq!(lines(game).len(), 1, "Chat Delay holds both");
                game.tick_chat_queue();
                let shown = lines(game);
                assert_eq!(shown.len(), 2, "{shown:?}");
                assert!(
                    shown[1].contains("hi") && !shown[1].contains("drop"),
                    "{shown:?}"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn direct_chat_sends_through_play_without_the_chat_screen_gate() {
    use mod_api::{
        ClientChatEvent, ClientHost, RegisterClientCommandsEvent, prelude::NetworkClientHost,
    };
    use sunrise_client::net::Session;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_protocol::packets::play::Serverbound as S;

    if run_fresh_cases(
        "cut_layout::game::remote::tests::chat_event_tests::direct_chat_sends_through_play_without_the_chat_screen_gate",
        "SUNRISE_DIRECT_CHAT_CASE",
        &["send"],
    ) {
        return;
    }
    struct Broken;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Broken {
        fn client_chat(&self, _: &mut dyn ClientHost, event: &mut ClientChatEvent<'_>) -> Flow {
            event.message.clear();
            panic!("broken chat handler");
        }
    }
    struct Listener;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn client_chat(&self, _: &mut dyn ClientHost, event: &mut ClientChatEvent<'_>) -> Flow {
            match event.original_message {
                "vetoed" => return Flow::Cancel,
                "cleared" => event.message.clear(),
                "rewrite" => event.message.push_str(" and send"),
                _ => {}
            }
            Flow::Continue
        }

        fn register_client_commands(
            &self,
            _: &mut dyn ClientHost,
            event: &mut RegisterClientCommandsEvent<'_>,
        ) {
            event
                .commands
                .push(mod_api::Command::new("local_only").executes_client(|_, _| Some(1)));
        }
    }
    let mods = vec![
        mod_api::ModEntry::new("broken_chat", Box::new(Broken)),
        mod_api::ModEntry::new("rewrite_chat", Box::new(Listener)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            assert!(NetworkClientHost::send_chat(game, "/warp spawn").is_err());
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ChatTest").unwrap();
            assert!(NetworkClientHost::send_chat(game, "/warp spawn").is_err());
            let remote = game.client.remote.as_mut().unwrap();
            remote.joined = true;
            remote.session = Session::replay("no writer");
            game.mark_message_processed(&[42; 256], true);
            assert!(NetworkClientHost::send_chat(game, "not connected").is_err());

            let (session, mut peer, _keepalive) = Session::packet_hold_probe();
            game.client.remote.as_mut().unwrap().session = session;
            let mut packet = || {
                let (id, body) = peer.recv().unwrap();
                S::decode(id, &body).unwrap()
            };
            let history = game.client.chat_ui.chat.recent.len();
            game.client.settings.chat_visibility = 2; // Hidden: UI is blocked, direct play send isn't.
            game.handle_chat_input("not sent by UI");
            game.handle_chat_input("/not_sent_by_ui");
            NetworkClientHost::send_chat(game, "vetoed").unwrap();
            NetworkClientHost::send_chat(game, "cleared").unwrap();
            NetworkClientHost::send_chat(game, "/local_only").unwrap();
            let overflow = "😀".repeat(129);
            assert!(NetworkClientHost::send_chat(game, &overflow).is_err());
            NetworkClientHost::send_chat(game, "rewrite").unwrap();
            let S::Chat(chat) = packet() else {
                panic!("expected the rewritten chat packet")
            };
            assert_eq!(chat.message, "rewrite and send");
            assert_eq!(
                chat.last_seen.offset, 1,
                "no rejected or canceled send advanced the window"
            );
            assert_eq!(game.client.chat_ui.chat.recent.len(), history);
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "broken_chat",
                    hook: "client_chat",
                }]
            );

            NetworkClientHost::send_chat(game, "/warp spawn").unwrap();
            assert_eq!(
                packet(),
                S::ChatCommand(play::ChatCommand {
                    command: "warp spawn".to_owned()
                })
            );

            game.client.settings.chat_visibility = 0;
            game.handle_chat_input(&overflow);
            let S::Chat(chat) = packet() else {
                panic!("expected screen-normalized chat")
            };
            assert_eq!(chat.message, "😀".repeat(128));
            assert_eq!(chat.last_seen.offset, 0);
        });
    });
}

/// A `client_chat` listener that resends and cancels the original reaches the hook again
/// only on the next event-loop wave, never from inside itself.
#[test]
#[ignore = "daily"]
fn a_client_chat_listener_that_resends_is_not_reentered() {
    use mod_api::{ClientChatEvent, ClientHost, prelude::NetworkClientHost};
    use sunrise_client::net::Session;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_protocol::packets::play::Serverbound as S;

    if run_fresh_cases(
        "cut_layout::game::remote::tests::chat_event_tests::a_client_chat_listener_that_resends_is_not_reentered",
        "SUNRISE_CHAT_RESEND_CASE",
        &["resend"],
    ) {
        return;
    }
    struct Resender;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Resender {
        fn client_chat(&self, host: &mut dyn ClientHost, event: &mut ClientChatEvent<'_>) -> Flow {
            if event.original_message == "again" {
                return Flow::Continue;
            }
            // Not a replacement through the event: a separate submission.
            let _ = NetworkClientHost::send_chat(host, "again");
            Flow::Cancel
        }
    }
    let mods = vec![mod_api::ModEntry::new("resend_chat", Box::new(Resender))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ChatTest").unwrap();
            game.client.remote.as_mut().unwrap().joined = true;
            let (session, mut peer, _keepalive) = Session::packet_hold_probe();
            game.client.remote.as_mut().unwrap().session = session;
            NetworkClientHost::send_chat(game, "original").unwrap();
            assert_eq!(game.client.native_client_commands.pending_chats, ["again"]);
            game.drain_pending_client_commands();
            let (id, body) = peer.recv().unwrap();
            let S::Chat(chat) = S::decode(id, &body).unwrap() else {
                panic!("expected the resent chat packet")
            };
            assert_eq!(chat.message, "again");
            assert!(game.client.native_client_commands.pending_chats.is_empty());
        });
    });
}
