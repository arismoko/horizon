#![cfg(test)]
use sunrise_client::spectator_menu::Outcome;
use sunrise_game_data::scoreboard::comp_text;
use sunrise_game_data::text::Text;
use sunrise_level::profile_lookup::SPECTATOR;
use sunrise_protocol::packets::play::Serverbound as S;
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_client::spectator_menu::Item;
    use sunrise_protocol::packets::{Empty, play, play::Clientbound as C};
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::in_own_process,
    };

    #[test]
    #[ignore = "daily"]
    fn an_unlisted_player_is_a_teleport_target_only_on_their_team_page() {
        in_own_process(
            "cut_layout::game::client_side::spectator::tests::an_unlisted_player_is_a_teleport_target_only_on_their_team_page",
            || {
                with_client_game(|game| {
                    let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                    // Java UUID(0, 0xcaff): hashCode 51967 mod 18 == 1 -> entity/player/slim/ari.
                    const UNLISTED: u128 = 0xcaff;
                    const LISTED: u128 = 0xbeef;
                    game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                        actions: 0x01 | 0x04 | 0x08, // ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED
                        entries: [
                            (UNLISTED, "Unlisted", 0, false),
                            (LISTED, "Listed", 0, true),
                            (0xaa, "Spectator", SPECTATOR, false),
                        ]
                        .into_iter()
                        .map(|(uuid, name, game_mode, listed)| play::PlayerInfo {
                            uuid,
                            name: Some(name.into()),
                            game_mode: Some(game_mode),
                            listed: Some(listed),
                            ..play::PlayerInfo::default()
                        })
                        .collect(),
                    }))
                    .unwrap();
                    let board = &mut game.score.board;
                    board.add_team("hidden");
                    for name in ["Unlisted", "Spectator", "Unknown"] {
                        board.add_to_team(name, "hidden");
                    }
                    board.add_team("visible");
                    board.add_to_team("Listed", "visible");
                    let expected_teams: Vec<Text> = board
                        .teams_streamed()
                        .iter()
                        .map(|t| comp_text(&t.display_name))
                        .collect();
                    assert_eq!(expected_teams.len(), 2);

                    let mut players = game.new_spectator_menu();
                    players.select_slot(0);
                    players.select_slot(0);
                    assert!(matches!(players.item(0), Item::Player(p) if p.uuid == LISTED));
                    assert_eq!(players.item(1), Item::Empty);

                    let mut teams = game.new_spectator_menu();
                    assert_eq!(teams.item(1), Item::TeleportToTeam { enabled: true });
                    teams.select_slot(1);
                    teams.select_slot(1);
                    let shown_teams: Vec<_> = [0, 1]
                        .into_iter()
                        .map(|slot| match teams.item(slot) {
                            Item::Team(team) => team,
                            other => panic!("expected team at slot {slot}, got {other:?}"),
                        })
                        .collect();
                    assert_eq!(
                        shown_teams
                            .iter()
                            .map(|t| t.name.clone())
                            .collect::<Vec<_>>(),
                        expected_teams
                    );
                    assert_eq!(teams.item(2), Item::Empty);
                    let hidden_slot = shown_teams
                        .iter()
                        .position(|t| t.name.plain() == "hidden")
                        .expect("hidden team present");
                    assert_eq!(shown_teams[hidden_slot].icon, "entity/player/slim/ari");
                    teams.select_slot(hidden_slot as i32);
                    teams.select_slot(hidden_slot as i32);
                    assert!(matches!(teams.item(0), Item::Player(p) if p.uuid == UNLISTED));
                    assert_eq!(teams.item(1), Item::Empty);
                    assert_eq!(teams.select_slot(0), Outcome::None);
                    game.spectator_outcome(teams.select_slot(0));
                    game.send(S::ClientTickEnd(Empty));
                    let mut teleports = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match S::decode(id, &body).unwrap() {
                            S::TeleportToEntity(p) => teleports.push(p.uuid),
                            S::ClientTickEnd(_) => break,
                            _ => {}
                        }
                    }
                    assert_eq!(teleports, [UNLISTED]);
                    let (tab, _, _) = game.remote_tab_players().unwrap();
                    assert_eq!(tab.iter().map(|p| p.uuid).collect::<Vec<_>>(), [LISTED]);
                });
            },
        );
    }
}
