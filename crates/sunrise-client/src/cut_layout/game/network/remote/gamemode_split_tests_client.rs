use crate::game::remote::abilities::gamemode_split_tests::SELF_NAME;
use crate::game::remote::abilities::gamemode_split_tests::SELF_UUID;
use crate::game::remote::abilities::gamemode_split_tests::abilities_packet;
use crate::game::remote::abilities::gamemode_split_tests::assert_row_core;
use crate::game::remote::abilities::gamemode_split_tests::drain_sent_abilities;
use crate::game::remote::abilities::gamemode_split_tests::install_stone_floor;
use crate::game::remote::abilities::gamemode_split_tests::login_packet;
use crate::game::remote::abilities::gamemode_split_tests::mode_id_from_name;
use crate::game::remote::abilities::gamemode_split_tests::oracle_witness;
use crate::game::remote::abilities::gamemode_split_tests::player_info_add_mode;
use crate::game::remote::abilities::gamemode_split_tests::position_packet;
use crate::game::remote::abilities::gamemode_split_tests::run_local_tick;
use crate::game::remote::abilities::gamemode_split_tests::spawn_info;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Screen;
use sunrise_protocol::connection::ConnReader;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;

pub(crate) fn run_resolved_player_info_retains_identity_across_remove_and_readd_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::resolved_player_info_retains_identity_across_remove_and_readd_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["resolved_player_info_retention"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (mut peer, _writer) = setup_client(game, 3, None, [0.5, 64.0, 0.5]);
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                let _ = drain_sent_abilities(game, &mut peer);

                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[0], true, Some(&sent));

                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 3))
                    .unwrap();
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[1], true, Some(&sent));

                game.handle_packet(C::PlayerInfoRemove(play::PlayerInfoRemove {
                    uuids: vec![SELF_UUID],
                }))
                .unwrap();
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[2], true, Some(&sent));

                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
                    .unwrap();
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[3], true, Some(&sent));

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn_info(2, None),
                    keep: 0,
                }))
                .unwrap();
                game.handle_packet(position_packet(2, [0.5, 64.0, 0.5]))
                    .unwrap();
                {
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.loaded_sent = true;
                    remote.level_load.finish();
                }
                game.client.screen = Screen::Playing;
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[4], true, Some(&sent));

                assert_eq!(rows.len(), 7);
                // #3: Respawn clears own_player_info and must leave it unresolved across Respawn
                // and PlayerPosition until the replacement player first reads it.
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn_info(0, None),
                    keep: 0,
                }))
                .unwrap();
                assert!(
                    game.client
                        .remote
                        .as_ref()
                        .unwrap()
                        .own_player_info
                        .get()
                        .is_none(),
                    "Respawn must leave the replacement player's PlayerInfo cache unresolved"
                );
                game.handle_packet(position_packet(3, [0.5, 64.0, 0.5]))
                    .unwrap();
                assert!(
                    game.client
                        .remote
                        .as_ref()
                        .unwrap()
                        .own_player_info
                        .get()
                        .is_none(),
                    "PlayerPosition must not eagerly resolve PlayerInfo before first read"
                );
                {
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.loaded_sent = true;
                    remote.level_load.finish();
                }
                game.client.screen = Screen::Playing;
                game.handle_packet(C::PlayerInfoRemove(play::PlayerInfoRemove {
                    uuids: vec![SELF_UUID],
                }))
                .unwrap();
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[5], true, Some(&sent));

                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 1))
                    .unwrap();
                run_local_tick(game);
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[6], true, Some(&sent));
            });
        },
    );
}

pub(crate) fn run_split_local_and_player_info_physics_matrix_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::split_local_and_player_info_physics_matrix_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["split_mode_physics_matrix"]
                .as_array()
                .unwrap();
            assert_eq!(rows.len(), 48);

            with_client_game(|game| {
                let (mut peer, _writer) = super::super::tests::connect_menu_test(game);
                peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();

                let modes = ["survival", "creative", "adventure", "spectator"];
                let info_modes: [Option<&str>; 5] = [
                    None,
                    Some("survival"),
                    Some("creative"),
                    Some("adventure"),
                    Some("spectator"),
                ];
                let mut row_idx = 0;

                for local in modes {
                    for info in info_modes {
                        for may_fly in [false, true] {
                            {
                                let remote = game.client.remote.as_mut().unwrap();
                                remote.profile = SELF_UUID;
                                remote.profile_name = SELF_NAME.into();
                                remote.tab_players.clear();
                                remote.tab_game_modes.clear();
                                remote.listed.clear();
                                remote.own_player_info = std::cell::OnceCell::new();
                                remote.has_logical_player = false;
                            }
                            game.player = None;
                            game.handle_packet(login_packet(83081, mode_id_from_name(local), None))
                                .unwrap();
                            install_stone_floor(game);
                            if let Some(info_name) = info {
                                game.handle_packet(player_info_add_mode(
                                    SELF_UUID,
                                    SELF_NAME,
                                    mode_id_from_name(info_name),
                                ))
                                .unwrap();
                            }
                            game.handle_packet(position_packet(1, [0.5, 64.0, 0.5]))
                                .unwrap();
                            game.player.as_mut().unwrap().ground.on_ground = true;
                            game.handle_packet(abilities_packet(
                                if may_fly { 0x04 } else { 0x00 },
                                0.05,
                                0.10,
                            ))
                            .unwrap();
                            {
                                let remote = game.client.remote.as_mut().unwrap();
                                remote.loaded_sent = true;
                                remote.level_load.finish();
                            }
                            game.client.screen = Screen::Playing;
                            let _ = drain_sent_abilities(game, &mut peer);

                            for _ in 0..3 {
                                run_local_tick(game);
                            }
                            let sent = drain_sent_abilities(game, &mut peer);
                            assert_row_core(game, &rows[row_idx], true, Some(&sent));
                            row_idx += 1;
                        }
                    }
                }

                // 8 landing rows: flying=true with downward velocity -0.1 into stone floor at y=64.0.
                for local in modes {
                    for info_name in ["adventure", "spectator"] {
                        {
                            let remote = game.client.remote.as_mut().unwrap();
                            remote.profile = SELF_UUID;
                            remote.profile_name = SELF_NAME.into();
                            remote.tab_players.clear();
                            remote.tab_game_modes.clear();
                            remote.listed.clear();
                            remote.own_player_info = std::cell::OnceCell::new();
                            remote.has_logical_player = false;
                        }
                        game.player = None;
                        game.handle_packet(login_packet(83081, mode_id_from_name(local), None))
                            .unwrap();
                        install_stone_floor(game);
                        game.handle_packet(player_info_add_mode(
                            SELF_UUID,
                            SELF_NAME,
                            mode_id_from_name(info_name),
                        ))
                        .unwrap();
                        game.handle_packet(position_packet(1, [0.5, 64.0, 0.5]))
                            .unwrap();
                        game.handle_packet(abilities_packet(0x06, 0.05, 0.10))
                            .unwrap();
                        game.player.as_mut().unwrap().velocity = Vec3::new(0.0, -0.1, 0.0);
                        {
                            let remote = game.client.remote.as_mut().unwrap();
                            remote.loaded_sent = true;
                            remote.level_load.finish();
                        }
                        game.client.screen = Screen::Playing;
                        let _ = drain_sent_abilities(game, &mut peer);

                        run_local_tick(game);
                        let sent = drain_sent_abilities(game, &mut peer);
                        assert_row_core(game, &rows[row_idx], true, Some(&sent));
                        row_idx += 1;
                    }
                }
                assert_eq!(row_idx, 48);
            });
        },
    );
}

pub(crate) fn setup_client(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    local_mode: i32,
    previous_mode: Option<i32>,
    pos: [f64; 3],
) -> (ConnReader, sunrise_protocol::connection::ConnWriter) {
    let (peer, writer) = super::super::tests::connect_menu_test(game);
    peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    {
        let remote = game.client.remote.as_mut().unwrap();
        remote.profile = SELF_UUID;
        remote.profile_name = SELF_NAME.into();
    }
    game.handle_packet(login_packet(83081, local_mode, previous_mode))
        .unwrap();
    install_stone_floor(game);
    game.handle_packet(position_packet(1, pos)).unwrap();
    {
        let remote = game.client.remote.as_mut().unwrap();
        remote.loaded_sent = true;
        remote.level_load.finish();
    }
    game.client.screen = Screen::Playing;
    (peer, writer)
}
