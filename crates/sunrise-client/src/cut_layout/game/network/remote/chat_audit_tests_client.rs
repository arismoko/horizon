use crate::game::remote::chat_audit_tests::ALICE;
use crate::game::remote::chat_audit_tests::BOB;
use crate::game::remote::chat_audit_tests::add_signed_sender;
use crate::game::remote::chat_audit_tests::deliver_signed;
use crate::game::remote::chat_audit_tests::player_chat;
use crate::game::remote::chat_audit_tests::take_seen_update;
use crate::game::remote::chat_audit_tests::tracker_offset;
use crate::game::remote::chat_audit_tests::with_delayed_chat;
use sunrise_client::chat::{ChatAbilities, ChatOptions, ChatVisibility};
use sunrise_protocol::rsa::RsaPrivateKey;
use {
    sunrise_protocol::packets::play, sunrise_protocol::packets::play::Clientbound as C,
    sunrise_protocol::secure_chat,
};

#[test]
#[ignore = "daily"]
fn delayed_player_chat_checks_block_filter_and_visibility_on_accept() {
    with_delayed_chat(
        "cut_layout::game::client_side::remote_client::chat_audit_tests::delayed_player_chat_checks_block_filter_and_visibility_on_accept",
        |game| {
            let key = RsaPrivateKey::generate(2048).unwrap();
            add_signed_sender(game, &key, ALICE);
            add_signed_sender(game, &key, BOB);
            let blocked = deliver_signed(game, &key, player_chat(ALICE, 0, "block me"));
            let mut packet = player_chat(BOB, 1, "filter me");
            packet.filter = (1, Vec::new());
            let fully_filtered = deliver_signed(game, &key, packet);
            let mut packet = player_chat(BOB, 2, "hello");
            packet.filter = (2, vec![0b0110]);
            let partially_filtered = deliver_signed(game, &key, packet);
            assert_eq!(tracker_offset(game), 0);
            assert!(game.client.chat_ui.chat.recent_lines(3).is_empty());
            assert_eq!(game.chat_queue_len(), 3);

            game.client.social.hidden.insert(ALICE);
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            let seen = take_seen_update(game);
            assert_eq!(seen.update.offset, 3);
            assert_eq!(seen.last_seen, vec![partially_filtered]);
            assert!(!seen.last_seen.contains(&blocked));
            assert!(!seen.last_seen.contains(&fully_filtered));
            let lines = game.client.chat_ui.chat.recent_lines(3);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].contains("h##lo"));

            let hidden = deliver_signed(game, &key, player_chat(BOB, 3, "hide with Chat option"));
            let font = &game.client.gui.atlas.font;
            game.client
                .chat_ui
                .chat
                .set_abilities(font, ChatAbilities::new(ChatVisibility::System));
            game.accept_next_delayed_message();
            let seen = take_seen_update(game);
            assert_eq!(seen.update.offset, 1);
            assert_eq!(seen.last_seen, vec![partially_filtered]);
            assert!(!seen.last_seen.contains(&hidden));
            assert_eq!(game.client.chat_ui.chat.recent_lines(3), lines);
        },
    );
}

#[test]
#[ignore = "daily"]
fn validation_error_acks_on_accept_and_checks_the_current_block_list() {
    with_delayed_chat(
        "cut_layout::game::client_side::remote_client::chat_audit_tests::validation_error_acks_on_accept_and_checks_the_current_block_list",
        |game| {
            sunrise_narrator::set_status(sunrise_narrator::Status::All);
            let prior_speech = sunrise_narrator::recent();
            let invalid = [6; secure_chat::SIGNATURE_BYTES];
            let mut packet = player_chat(ALICE, 0, "sender missing from player info");
            packet.signature = Some(invalid.to_vec());
            game.handle_packet(C::PlayerChat(packet)).unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            assert_eq!(tracker_offset(game), 0);

            // Even with an invalid signature to acknowledge, a validation error has no queue key.
            game.handle_packet(C::DeleteChat(play::DeleteChat {
                signature: (0, None),
            }))
            .unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            game.client.social.hidden.insert(ALICE);
            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            let seen = take_seen_update(game);
            assert_eq!(seen.update.offset, 1);
            assert!(seen.last_seen.is_empty());
            assert!(game.client.chat_ui.chat.recent_lines(1).is_empty());
            assert_eq!(sunrise_narrator::recent(), prior_speech);

            // An unhidden invalid message shows and narrates on acceptance, still acked as not shown.
            let invalid_visible = [7; secure_chat::SIGNATURE_BYTES];
            let mut packet = player_chat(BOB, 1, "unhidden sender missing");
            packet.signature = Some(invalid_visible.to_vec());
            game.handle_packet(C::PlayerChat(packet)).unwrap();
            assert_eq!(game.chat_queue_len(), 1);
            assert_eq!(tracker_offset(game), 0);
            assert!(game.client.chat_ui.chat.recent_lines(1).is_empty());
            assert_eq!(sunrise_narrator::recent(), prior_speech);

            game.tick_chat_queue();
            assert_eq!(game.chat_queue_len(), 0);
            let seen = take_seen_update(game);
            assert_eq!(seen.update.offset, 1);
            assert!(seen.last_seen.is_empty());
            assert_eq!(game.client.chat_ui.chat.recent_lines(1).len(), 1);
            assert_ne!(sunrise_narrator::recent(), prior_speech);
            let chat = &mut game.client.chat_ui.chat;
            let font = &game.client.gui.atlas.font;
            chat.options = ChatOptions::default();
            chat.smooth = false;
            chat.hide_indicator = false;
            let (tag, _) = chat.focused_tags(font, 100, 1.0, 59.0);
            assert_eq!(tag.unwrap().translation_key(), Some("chat.tag.error"));
        },
    );
}
