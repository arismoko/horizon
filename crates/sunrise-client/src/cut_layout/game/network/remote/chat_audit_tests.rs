#![cfg(feature = "client")]
use sunrise_protocol::rsa::{RsaPrivateKey, RsaPublicKey, SignatureHash};
use sunrise_protocol::secure_chat;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::in_own_process,
};
use {
    sunrise_client::chat::ChatAbilities, sunrise_client::chat::ChatOptions,
    sunrise_client::chat::ChatVisibility, sunrise_client::hud_layout::GuiDraw,
    sunrise_client::options_file::V, sunrise_level::profile_lookup::TabPlayer,
};
use {
    sunrise_gameplay_common::game::remote::C, sunrise_gameplay_common::game::remote::Signature,
    sunrise_gameplay_common::game::remote::Tag, sunrise_gameplay_common::game::remote::play,
};

pub(crate) const ALICE: u128 = 10;
pub(crate) const BOB: u128 = 11;
pub(crate) const SESSION: u128 = 42;

pub(crate) fn with_delayed_chat(
    test: &str,
    check: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    // The game fixture registers keys and particle providers; keep them in the child.
    in_own_process(test, || {
        with_client_game(|game| {
            game.init_remote_for_replay().unwrap();
            game.client.remote.as_mut().unwrap().has_logical_player = true;
            game.client.settings.chat_delay = 1.0;
            check(game);
        });
    });
}

pub(crate) fn tracker_offset(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> i32 {
    game.client
        .remote
        .as_mut()
        .unwrap()
        .secure_chat
        .tracker_mut()
        .offset()
}

pub(crate) fn take_seen_update(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> secure_chat::SeenUpdate {
    game.client
        .remote
        .as_mut()
        .unwrap()
        .secure_chat
        .tracker_mut()
        .update()
}

pub(crate) fn bound() -> play::ChatTypeBound {
    play::ChatTypeBound {
        chat_type: 0,
        name: Tag::String("Alice".into()),
        target: None,
    }
}

pub(crate) fn player_chat(sender: u128, global_index: i32, content: &str) -> play::PlayerChat {
    play::PlayerChat {
        global_index,
        sender,
        index: global_index,
        signature: None,
        content: content.to_owned(),
        timestamp: sunrise_java_properties::now_millis(),
        salt: 0,
        last_seen: Vec::new(),
        unsigned_content: None,
        filter: (0, Vec::new()),
        chat_type: bound(),
    }
}

pub(crate) fn add_sender(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, sender: u128) {
    game.client
        .remote
        .as_mut()
        .unwrap()
        .tab_players
        .insert(sender, TabPlayer::new(sender, "Player"));
}

pub(crate) fn add_signed_sender(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    key: &RsaPrivateKey,
    sender: u128,
) {
    add_sender(game, sender);
    // A fixture key signs both its profile certificate and its chat packets;
    // the packet handler still runs the real keyed validator and message chain.
    let der = key.public_der();
    let expires_at = i64::MAX;
    let payload = [&sender.to_be_bytes()[..], &expires_at.to_be_bytes(), &der].concat();
    let profile_key = secure_chat::ProfileKey {
        expires_at,
        key: der.clone(),
        signature: key.sign(SignatureHash::Sha1, &payload).unwrap(),
    };
    let services_key = RsaPublicKey::from_der(&der).unwrap();
    let session =
        secure_chat::ChatSession::validate(SESSION, profile_key, &[services_key], sender).unwrap();
    game.client
        .remote
        .as_mut()
        .unwrap()
        .chat_validators
        .insert(sender, secure_chat::MessageValidator::keyed(session));
}

pub(crate) fn deliver_signed(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    key: &RsaPrivateKey,
    mut packet: play::PlayerChat,
) -> Signature {
    let body = secure_chat::Body {
        content: &packet.content,
        timestamp: packet.timestamp,
        salt: packet.salt,
        last_seen: &[],
    };
    let payload = secure_chat::signed_payload(packet.sender, SESSION, packet.index, &body);
    let bytes = key.sign(SignatureHash::Sha256, &payload).unwrap();
    let signature = secure_chat::signature(&bytes).unwrap();
    packet.signature = Some(bytes);
    game.handle_packet(C::PlayerChat(packet)).unwrap();
    signature
}

#[test]
#[ignore = "daily"]
fn disguised_chat_has_a_system_tag_but_is_player_chat() {
    with_delayed_chat(
        "cut_layout::game::remote::chat_audit_tests::disguised_chat_has_a_system_tag_but_is_player_chat",
        |game| {
            game.client
                .settings
                .file
                .set("onlyShowSecureChat", V::Bool(true));

            // Before a logical local player exists, disguised chat is rejected on accept.
            game.client.remote.as_mut().unwrap().has_logical_player = false;
            game.handle_packet(C::DisguisedChat(play::DisguisedChat {
                message: Tag::String("before login".into()),
                chat_type: bound(),
            }))
            .unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            assert!(game.chat_ui_for_test().chat.recent_lines(1).is_empty());
            game.client.remote.as_mut().unwrap().has_logical_player = true;

            game.handle_packet(C::SystemChat(play::SystemChat {
                content: Tag::String("server".into()),
                overlay: false,
            }))
            .unwrap();
            game.handle_packet(C::DisguisedChat(play::DisguisedChat {
                message: Tag::String("disguised".into()),
                chat_type: bound(),
            }))
            .unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            assert_eq!(game.chat_ui_for_test().chat.recent_lines(2), ["server"]);
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            assert!(game.chat_ui_for_test().chat.recent_lines(1)[0].contains("disguised"));
            assert_eq!(tracker_offset(game), 0);

            let (chat_ui, font) = game.chat_ui_and_font_for_test();
            let chat = &mut chat_ui.chat;
            chat.options = ChatOptions::default();
            chat.smooth = false;
            chat.hide_indicator = false;
            // At the default scale the newest row is just above y=60, and the
            // second row is 9 px above it. x=1 lands on the indicator bar.
            let (disguised, _) = chat.focused_tags(font, 100, 1.0, 59.0);
            let (server, _) = chat.focused_tags(font, 100, 1.0, 50.0);
            assert_eq!(
                disguised.unwrap().translation_key(),
                Some("chat.tag.system")
            );
            assert_eq!(
                server.unwrap().translation_key(),
                Some("chat.tag.system_single_player")
            );
            assert!(chat.focused_tags(font, 100, 5.0, 59.0).1.is_empty());
            assert!(chat.layout(100, 0.0, true).iter().any(|draw| matches!(
                draw,
                GuiDraw::Fill {
                    x0: -4,
                    x1: -2,
                    y1: 60,
                    color: 0xffd0d0d0,
                    ..
                }
            )));

            chat.set_abilities(font, ChatAbilities::new(ChatVisibility::System));
            let visible = chat
                .layout(100, 0.0, true)
                .into_iter()
                .filter_map(|draw| match draw {
                    GuiDraw::Text { text, .. } => Some(text.plain()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                visible,
                ["server"],
                "disguised chat retains the player source"
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn delete_chat_removes_a_queued_fully_filtered_message_without_ack() {
    with_delayed_chat(
        "cut_layout::game::remote::chat_audit_tests::delete_chat_removes_a_queued_fully_filtered_message_without_ack",
        |game| {
            let key = RsaPrivateKey::generate(2048).unwrap();
            add_signed_sender(game, &key, ALICE);
            let mut packet = player_chat(ALICE, 0, "filtered then deleted");
            packet.filter = (1, Vec::new());
            deliver_signed(game, &key, packet);
            assert_eq!(game.chat_queue_len(), 1);
            assert_eq!(tracker_offset(game), 0);

            // The most recent signed packet supplied this cached signature.
            game.handle_packet(C::DeleteChat(play::DeleteChat {
                signature: (0, None),
            }))
            .unwrap();
            assert_eq!(game.chat_queue_len(), 0);
            game.tick_chat_queue();
            let seen = take_seen_update(game);
            assert_eq!(seen.update.offset, 0);
            assert!(seen.last_seen.is_empty());
            assert!(game.chat_ui_for_test().chat.recent_lines(1).is_empty());
        },
    );
}

#[test]
#[ignore = "daily"]
fn delayed_chat_uses_only_show_secure_chat_from_receipt() {
    with_delayed_chat(
        "cut_layout::game::remote::chat_audit_tests::delayed_chat_uses_only_show_secure_chat_from_receipt",
        |game| {
            add_sender(game, ALICE);
            game.client
                .settings
                .file
                .set("onlyShowSecureChat", V::Bool(true));
            game.handle_packet(C::PlayerChat(player_chat(ALICE, 0, "unsigned, hidden")))
                .unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            game.client
                .settings
                .file
                .set("onlyShowSecureChat", V::Bool(false));
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            assert!(game.chat_ui_for_test().chat.recent_lines(1).is_empty());

            game.handle_packet(C::PlayerChat(player_chat(ALICE, 1, "unsigned, visible")))
                .unwrap();
            game.client
                .settings
                .file
                .set("onlyShowSecureChat", V::Bool(true));
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            assert!(game.chat_ui_for_test().chat.recent_lines(1)[0].contains("unsigned, visible"));

            // Language changes while queued re-evaluate decoration and trust on acceptance.
            game.client
                .settings
                .file
                .set("onlyShowSecureChat", V::Bool(false));
            let key = RsaPrivateKey::generate(2048).unwrap();
            add_signed_sender(game, &key, BOB);
            sunrise_game_data::lang::set(sunrise_game_data::lang::Language::of(
                "test_before",
                [
                    ("chat.type.text".into(), "<%s> %s".into()),
                    ("test.delayed_msg".into(), "before".into()),
                ],
            ));
            let mut packet = player_chat(BOB, 2, "after");
            packet.unsigned_content = Some(Tag::Compound(
                [("translate".into(), Tag::String("test.delayed_msg".into()))].into(),
            ));
            deliver_signed(game, &key, packet);
            assert_eq!(game.chat_queue_len(), 1);
            sunrise_game_data::lang::set(sunrise_game_data::lang::Language::of(
                "test_after",
                [
                    ("chat.type.text".into(), "<%s> %s".into()),
                    ("test.delayed_msg".into(), "after".into()),
                ],
            ));
            game.accept_next_delayed_message();
            assert_eq!(game.chat_queue_len(), 0);
            let newest = game.chat_ui_for_test().chat.recent_lines(1);
            assert!(newest[0].contains("after"), "{newest:?}");
            let (chat_ui, font) = game.chat_ui_and_font_for_test();
            let chat = &mut chat_ui.chat;
            chat.options = ChatOptions::default();
            chat.smooth = false;
            chat.hide_indicator = false;
            assert!(chat.focused_tags(font, 100, 1.0, 59.0).0.is_none());
        },
    );
}
