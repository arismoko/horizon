#![cfg(feature = "client")]
use sunrise_core::nbt::Tag;
use sunrise_game_data::scoreboard::comp_text;
use sunrise_protocol::packets::play::{Clientbound, SetPlayerTeam, TeamParameters};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::in_own_process,
};

fn send_team(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    name: &str,
    method: i8,
    display: Option<&str>,
    players: &[&str],
) {
    let packet = Clientbound::SetPlayerTeam(SetPlayerTeam {
        name: name.into(),
        method,
        parameters: display.map(|name| TeamParameters {
            display_name: Tag::String(name.into()),
            prefix: Tag::String("".into()),
            suffix: Tag::String("".into()),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: None,
            options: 0,
        }),
        players: players.iter().map(|&player| player.into()).collect(),
    });
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    let id = reader.varint().unwrap();
    game.handle_packet(Clientbound::decode(id, reader.rest()).unwrap())
        .unwrap();
}

#[test]
#[ignore = "daily"]
fn unknown_team_packets_leave_scoreboard_unchanged() {
    in_own_process(
        "cut_layout::game::client_side::remote_client::sync_reads::team_packet_tests::unknown_team_packets_leave_scoreboard_unchanged",
        || {
            with_client_game(|game| {
                send_team(game, "stable", 0, Some("Stable"), &["member"]);
                for (method, display, players) in [
                    (1, None, &[][..]),
                    (2, Some("Unknown"), &[]),
                    (4, None, &["member"]),
                    (3, None, &["member", "phantom"]),
                ] {
                    send_team(game, "delayed", method, display, players);
                    let board = &game.score.board;
                    assert_eq!(board.teams_streamed().len(), 1, "method {method}");
                    assert!(board.team("delayed").is_none(), "method {method}");
                    assert_eq!(
                        board.players_team("member").map(|team| team.name.as_str()),
                        Some("stable"),
                        "method {method} moved an existing member"
                    );
                    let stable = board.team("stable").unwrap();
                    assert!(stable.players.contains("member"), "method {method}");
                    assert_eq!(comp_text(&stable.display_name).plain(), "Stable");
                    assert!(
                        !board.team_members().any(|p| p == "phantom"),
                        "method {method}"
                    );
                }

                send_team(game, "delayed", 0, Some("Delayed"), &[]);
                let delayed = game.score.board.team("delayed").unwrap();
                assert!(delayed.players.iter().next().is_none());
                assert!(game.score.board.players_team("phantom").is_none());
                send_team(game, "delayed", 2, Some("Renamed"), &[]);
                let delayed = game.score.board.team("delayed").unwrap();
                assert_eq!(comp_text(&delayed.display_name).plain(), "Renamed");

                send_team(game, "delayed", 3, None, &["member", "other"]);
                let board = &game.score.board;
                assert_eq!(board.players_team("member").unwrap().name, "delayed");
                assert!(!board.team("stable").unwrap().players.contains("member"));

                send_team(game, "delayed", 4, None, &["member"]);
                let board = &game.score.board;
                assert!(board.players_team("member").is_none());
                assert!(!board.team("delayed").unwrap().players.contains("member"));

                assert_eq!(board.players_team("other").unwrap().name, "delayed");
                send_team(game, "delayed", 1, None, &[]);
                let board = &game.score.board;
                assert!(board.team("delayed").is_none());
                assert!(board.players_team("other").is_none());
                assert_eq!(board.teams_streamed().len(), 1);
            });
        },
    );
}
