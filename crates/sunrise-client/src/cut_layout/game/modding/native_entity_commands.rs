#![cfg(test)]
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{GameMode, PermissionLevel};
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_protocol::packets::play::Clientbound as C;

    fn with_command_guests(
        name: &str,
        body: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        sunrise_gameplay_common::game::in_own_process(
            &format!("cut_layout::game::server_side::native_entity_commands::tests::{name}"),
            || {
                with_guests(body);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn silent_entity_say_reaches_every_guest_without_feedback_or_outer_capture() {
        let name = "silent_entity_say_reaches_every_guest_without_feedback_or_outer_capture";
        with_command_guests(name, |game| {
            game.build_registries();
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let level = game.level(game.dim.id()).unwrap();
            let stand = game
                .level_summon(
                    level,
                    "minecraft:armor_stand",
                    [1.5, 70.0, 1.5],
                    Default::default(),
                )
                .unwrap()
                .unwrap();
            let receivers: Vec<_> = [12, 13]
                .into_iter()
                .map(|conn| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(conn, |guest| {
                            guest.sender =
                                sunrise_sim::lan::PlayerSender::new(tx, std::sync::Arc::default());
                        });
                    rx
                })
                .collect();
            game.as_guest(12, |game| {
                game.server.chat_capture = Some(vec![Text::literal("outer command")]);
                let denied = stand.run_command_with_output(
                    game,
                    "/say forbidden",
                    PermissionLevel::from_level(0),
                    false,
                );
                assert!(!denied.success);
                assert!(denied.feedback.is_empty());
                for rx in &receivers {
                    assert!(rx.try_recv().is_err(), "denied say must not broadcast");
                }
                let result = stand.run_command_with_output(
                    game,
                    "/say silent source announcement",
                    PermissionLevel::from_level(2),
                    false,
                );
                assert!(result.success);
                assert!(result.feedback.is_empty());
                assert_eq!(
                    game.server
                        .chat_capture
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(Text::plain)
                        .collect::<Vec<_>>(),
                    ["outer command"]
                );
                for rx in &receivers {
                    let mut announcements = 0;
                    while let Ok(sunrise_sim::lan::Out::Packet(body)) = rx.try_recv() {
                        let mut reader = sunrise_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let C::DisguisedChat(chat) = C::decode(id, reader.rest()).unwrap() {
                            assert_eq!(
                                sunrise_game_data::component::name_text(&chat.message).plain(),
                                "silent source announcement"
                            );
                            announcements += 1;
                        }
                    }
                    assert_eq!(announcements, 1, "acting guest and peer each hear say once");
                }
            })
            .unwrap();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_command_returns_permission_failures_and_success_without_swallowing_chat() {
        let name = "player_command_returns_permission_failures_and_success_without_swallowing_chat";
        with_command_guests(name, |game| {
            game.build_registries();
            let player = game.player(12).unwrap();
            assert_eq!(player.game_mode(game), Some(GameMode::Survival));
            for permission in [0, 2] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.permission = permission)
                    .unwrap();
                game.as_guest(12, |game| {
                    let level = PermissionLevel::from_level(i32::from(
                        player.permission_level(game).unwrap(),
                    ));
                    let before = game.server.guest_packets.len();
                    // A surrounding command's output must stay separate from this run.
                    game.server.chat_capture = Some(vec![Text::literal("outer command")]);
                    let result = player.entity().run_command_with_output(
                        game,
                        "/gamemode creative",
                        level,
                        false,
                    );
                    assert_eq!(result.success, permission == 2);
                    if permission == 0 {
                        assert_eq!(
                            result.feedback,
                            [
                                "Unknown or incomplete command. See below for error",
                                "gamemode creative<--[HERE]",
                            ]
                        );
                        assert_eq!(player.game_mode(game), Some(GameMode::Survival));
                    } else {
                        assert_eq!(result.feedback, ["Set own game mode to Creative Mode"]);
                        assert_eq!(player.game_mode(game), Some(GameMode::Creative));
                    }
                    let chat: Vec<_> = game.server.guest_packets[before..]
                        .iter()
                        .filter_map(|packet| match packet {
                            C::SystemChat(chat) if !chat.overlay => {
                                Some(sunrise_game_data::component::name_text(&chat.content).plain())
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(chat, result.feedback);
                    let outer = game.server.chat_capture.take().unwrap();
                    assert_eq!(outer.len(), 1);
                    assert_eq!(outer[0].plain(), "outer command");
                })
                .unwrap();
            }
            assert_eq!(player.game_mode(game), Some(GameMode::Creative));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn suppressed_player_command_changes_game_mode_without_telling_player_or_ops() {
        let name = "suppressed_player_command_changes_game_mode_without_telling_player_or_ops";
        with_command_guests(name, |game| {
            use std::sync::{Arc, mpsc};
            use sunrise_sim::lan::{Out, PlayerSender};

            game.build_registries();
            let player = game.player(12).unwrap();
            let entity = player.entity();
            let (tx, rx) = mpsc::channel();
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.ops.insert(13, 2);
            hub.update_guest(13, |guest| {
                guest.permission = 2;
                guest.sender = PlayerSender::new(tx, Arc::default());
            })
            .unwrap();
            let op_chat_count = || {
                rx.try_iter()
                    .filter(|out| {
                        let Out::Packet(body) = out else { return false };
                        let mut reader = sunrise_protocol::buf::Reader::new(body);
                        let id = reader.varint().unwrap();
                        matches!(C::decode(id, reader.rest()).unwrap(), C::SystemChat(_))
                    })
                    .count()
            };
            game.as_guest(12, |game| {
                let loud = entity.run_command_with_output(
                    game,
                    "gamemode creative",
                    PermissionLevel::Gamemasters,
                    false,
                );
                assert!(loud.success && !loud.feedback.is_empty());
                assert_eq!(player.game_mode(game), Some(GameMode::Creative));
                assert_eq!(
                    op_chat_count(),
                    1,
                    "the operator hears unsuppressed feedback"
                );

                let before = game.server.guest_packets.len();
                let quiet = entity.run_command_with_output(
                    game,
                    "gamemode survival",
                    PermissionLevel::Gamemasters,
                    true,
                );
                assert!(quiet.success);
                assert!(quiet.feedback.is_empty());
                assert_eq!(player.game_mode(game), Some(GameMode::Survival));
                assert!(!entity.run_command(game, "gamemode creative", PermissionLevel::All, true));
                assert!(
                    !game.server.guest_packets[before..]
                        .iter()
                        .any(|p| matches!(p, C::SystemChat(_)))
                );
                assert_eq!(op_chat_count(), 0);
            })
            .unwrap();
        });
    }
}
