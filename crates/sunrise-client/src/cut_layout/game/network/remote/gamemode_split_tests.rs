//! Parity tests for Minecraft 26.3 client local-game-mode vs `PlayerInfo` split (Item 3).
//! Witness recorded into `.local` by `tools/test_gamemode_split_parity.py` (`VanillaClientGameModeSplitOracle`, JDK 25).
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use serde_json::Value as JsonValue;
use sunrise_command::args::GameMode;
use sunrise_gameplay_common::game::remote::{C, Chunk, Empty, S, Screen, Vec3, play};
use sunrise_gameplay_common::game_modes::mode_from_id;
use sunrise_protocol::connection::ConnReader;

pub(crate) const SELF_UUID: u128 = 0xdf71_dcc7_da3b_3506_b229_6d1f_589e_2569;
pub(crate) const SELF_NAME: &str = "BX0W";

pub(crate) fn oracle_witness() -> JsonValue {
    let path = std::env::var("SUNRISE_GAMEMODE_SPLIT_WITNESS").unwrap_or_else(|_| {
        let fallback = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/witness/26.3/gamemode_split/witness.json");
        assert!(
            fallback.is_file(),
            "set SUNRISE_GAMEMODE_SPLIT_WITNESS or run tools/test_gamemode_split_parity.py"
        );
        fallback.to_string_lossy().into_owned()
    });
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_GAMEMODE_SPLIT_WITNESS={path} (run through tools/test_gamemode_split_parity.py): {err}"
        )
    });
    serde_json::from_str(&raw).expect("valid 26.3 gamemode split witness JSON")
}

pub(crate) fn f32_bits(hex: &JsonValue) -> u32 {
    u32::from_str_radix(hex.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

pub(crate) fn f64_bits(hex: &JsonValue) -> u64 {
    u64::from_str_radix(hex.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

pub(crate) fn mode_id_from_name(name: &str) -> i32 {
    GameMode::from_name(name).unwrap().id()
}

pub(crate) fn spawn_info(game_type: i32, previous_game_type: Option<i32>) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type,
        previous_game_type,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: -63,
    }
}

pub(crate) fn login_packet(player_id: i32, game_type: i32, previous_game_type: Option<i32>) -> C {
    C::Login(play::Login {
        player_id,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn_info(game_type, previous_game_type),
        online_mode: false,
        enforces_secure_chat: false,
    })
}

pub(crate) fn position_packet(teleport: i32, position: [f64; 3]) -> C {
    C::PlayerPosition(play::PlayerPosition {
        teleport,
        position,
        delta: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        relatives: 0,
    })
}

pub(crate) fn abilities_packet(flags: i8, flying_speed: f32, walking_speed: f32) -> C {
    C::PlayerAbilities(play::PlayerAbilities {
        flags,
        flying_speed,
        walking_speed,
    })
}

pub(crate) fn change_mode_packet(param: f32) -> C {
    C::GameEvent(play::GameEvent { event: 3, param })
}

pub(crate) fn player_info_add_mode(uuid: u128, name: &str, game_mode: i32) -> C {
    C::PlayerInfoUpdate(play::PlayerInfoUpdate {
        actions: 0x01 | 0x04 | 0x08,
        entries: vec![play::PlayerInfo {
            uuid,
            name: Some(name.into()),
            game_mode: Some(game_mode),
            listed: Some(true),
            ..play::PlayerInfo::default()
        }],
    })
}

pub(crate) fn player_info_mode(uuid: u128, game_mode: i32) -> C {
    C::PlayerInfoUpdate(play::PlayerInfoUpdate {
        actions: 0x04,
        entries: vec![play::PlayerInfo {
            uuid,
            game_mode: Some(game_mode),
            ..play::PlayerInfo::default()
        }],
    })
}

pub(crate) fn install_stone_floor(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.client
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..=1 {
        for z in 0..=1 {
            for y in 62..=63 {
                game.world.set(x, y, z, stone).unwrap();
            }
        }
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::abilities::gamemode_split_tests::setup_client;

pub(crate) fn drain_sent_abilities(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    peer: &mut ConnReader,
) -> Vec<bool> {
    game.send(S::ClientTickEnd(Empty));
    let mut sent = Vec::new();
    while let Ok((id, bytes)) = peer.recv() {
        match S::decode(id, &bytes) {
            Ok(S::ClientTickEnd(_)) => break,
            Ok(S::PlayerAbilities(a)) => sent.push(a.flags & 2 != 0),
            _ => {}
        }
    }
    sent
}

pub(crate) fn run_local_tick(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.tick_local_player(Vec::new(), 7, None).unwrap();
}

pub(crate) fn player_info_mode_name(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
) -> Option<&'static str> {
    let r = game.client.remote.as_ref()?;
    let t = r.tab_players.get(&r.profile)?;
    Some(mode_from_id(t.game_mode).name())
}

pub(crate) fn assert_row_core(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    row: &JsonValue,
    check_pos_vel: bool,
    sent_abilities: Option<&[bool]>,
) {
    let label = row["step"].as_str().unwrap();
    let local_ref = game.local_player().expect("local_player handle");
    let ab = local_ref.abilities(game).expect("client abilities");
    let p = game.player.as_ref().expect("local player present");
    let want_ab = &row["abilities"];

    assert_eq!(
        game.mode.name(),
        row["local_game_mode"].as_str().unwrap(),
        "{label}: local_game_mode"
    );
    assert_eq!(
        game.previous_mode.map(GameMode::name),
        row["previous_local_game_mode"].as_str(),
        "{label}: previous_local_game_mode"
    );
    assert_eq!(
        player_info_mode_name(game),
        row["player_info_game_mode"].as_str(),
        "{label}: player_info_game_mode"
    );
    assert_eq!(
        game.player_game_mode(local_ref).map(GameMode::name),
        row["player_game_mode"].as_str(),
        "{label}: player_game_mode"
    );
    let want_spectator = row["is_spectator"].as_bool().unwrap();
    let want_creative = row["is_creative"].as_bool().unwrap();
    let want_no_physics = row["no_physics"].as_bool().unwrap();
    assert_eq!(
        game.own_player_is_spectator(),
        want_spectator,
        "{label}: own_player_is_spectator"
    );
    assert_eq!(
        game.player_is_spectator(local_ref),
        Some(want_spectator),
        "{label}: player_is_spectator"
    );
    assert_eq!(
        game.own_player_is_creative(),
        want_creative,
        "{label}: own_player_is_creative"
    );
    assert_eq!(
        game.player_is_creative(local_ref),
        Some(want_creative),
        "{label}: player_is_creative"
    );
    assert_eq!(p.spectator, want_no_physics, "{label}: no_physics");
    assert_eq!(
        p.ground.on_ground,
        row["on_ground"].as_bool().unwrap(),
        "{label}: on_ground"
    );
    assert_eq!(
        (p.invulnerable, ab.invulnerable),
        (
            want_ab["invulnerable"].as_bool().unwrap(),
            want_ab["invulnerable"].as_bool().unwrap()
        ),
        "{label}: abilities.invulnerable"
    );
    assert_eq!(
        (p.flying, ab.flying),
        (
            want_ab["flying"].as_bool().unwrap(),
            want_ab["flying"].as_bool().unwrap()
        ),
        "{label}: abilities.flying"
    );
    assert_eq!(
        (p.may_fly, ab.may_fly),
        (
            want_ab["may_fly"].as_bool().unwrap(),
            want_ab["may_fly"].as_bool().unwrap()
        ),
        "{label}: abilities.may_fly"
    );
    assert_eq!(
        (game.instabuild(), ab.instabuild),
        (
            want_ab["instabuild"].as_bool().unwrap(),
            want_ab["instabuild"].as_bool().unwrap()
        ),
        "{label}: abilities.instabuild"
    );
    assert_eq!(
        (game.may_build(), ab.may_build),
        (
            want_ab["may_build"].as_bool().unwrap(),
            want_ab["may_build"].as_bool().unwrap()
        ),
        "{label}: abilities.may_build"
    );
    let want_fly_bits = f32_bits(&want_ab["fly_speed_bits"]);
    let want_walk_bits = f32_bits(&want_ab["walk_speed_bits"]);
    assert_eq!(
        (p.flying_speed.to_bits(), ab.fly_speed.to_bits()),
        (want_fly_bits, want_fly_bits),
        "{label}: abilities.fly_speed_bits"
    );
    assert_eq!(
        (
            game.remote_walking_speed().to_bits(),
            ab.walk_speed.to_bits()
        ),
        (want_walk_bits, want_walk_bits),
        "{label}: abilities.walk_speed_bits"
    );
    if check_pos_vel {
        let want_pos = row["pos_bits"].as_array().unwrap();
        let got_pos = [
            p.position.x.to_bits(),
            p.position.y.to_bits(),
            p.position.z.to_bits(),
        ];
        let want_pos = [
            f64_bits(&want_pos[0]),
            f64_bits(&want_pos[1]),
            f64_bits(&want_pos[2]),
        ];
        assert_eq!(got_pos, want_pos, "{label}: pos_bits");

        let want_vel = row["vel_bits"].as_array().unwrap();
        let got_vel = [
            p.velocity.x.to_bits(),
            p.velocity.y.to_bits(),
            p.velocity.z.to_bits(),
        ];
        let want_vel = [
            f64_bits(&want_vel[0]),
            f64_bits(&want_vel[1]),
            f64_bits(&want_vel[2]),
        ];
        assert_eq!(got_vel, want_vel, "{label}: vel_bits");
    }
    if let Some(got_sent) = sent_abilities {
        let want_sent: Vec<bool> = row["sent_abilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        assert_eq!(got_sent, want_sent.as_slice(), "{label}: sent_abilities");
    }
}

/// Real 26.3 oracle witness (`scenarios.genpvp_packet_sequence`, recorded by `tools/test_gamemode_split_parity.py`):
/// step-by-step replay of the genpvp packet sequence plus 5 ticks on stone, verifying every
/// mode, ability (including `may_build`), physics flag, and exact position/velocity bits.
#[test]
#[ignore = "daily"]
fn genpvp_packet_sequence_matches_26_3_oracle_step_by_step() {
    run_genpvp_packet_sequence_matches_26_3_oracle_step_by_step();
}

pub(crate) fn run_genpvp_packet_sequence_matches_26_3_oracle_step_by_step() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::genpvp_packet_sequence_matches_26_3_oracle_step_by_step",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["genpvp_packet_sequence"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (mut peer, _writer) = setup_client(game, 1, None, [0.5, 64.0, 0.5]);
                let _ = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[0], true, Some(&[]));

                let packets = [
                    abilities_packet(0x0d, 0.05, 0.10),
                    player_info_add_mode(SELF_UUID, SELF_NAME, 1),
                    abilities_packet(0x00, 0.05, 0.10),
                    player_info_mode(SELF_UUID, 2),
                    change_mode_packet(2.0),
                    abilities_packet(0x00, 0.05, 0.10),
                    change_mode_packet(1.0),
                    abilities_packet(0x0d, 0.0, 0.0),
                    change_mode_packet(3.0),
                    abilities_packet(0x00, 0.0, 0.0),
                ];
                for (idx, pkt) in packets.into_iter().enumerate() {
                    game.handle_packet(pkt).unwrap();
                    let sent = drain_sent_abilities(game, &mut peer);
                    assert_row_core(game, &rows[idx + 1], true, Some(&sent));
                }
                for tick in 0..5 {
                    run_local_tick(game);
                    let sent = drain_sent_abilities(game, &mut peer);
                    assert_row_core(game, &rows[11 + tick], true, Some(&sent));
                }
            });
        },
    );
}

/// Real 26.3 oracle witness (`scenarios.change_game_mode_abilities`):
/// `CHANGE_GAME_MODE` calls only `MultiPlayerGameMode.setLocalMode` + `GameType.updatePlayerAbilities`
/// — never resetting `flying_speed`, never forcing `flying` on `Spectator -> Creative`, never
/// running server-side `in_range_of_ground` landing, and applying in exact packet order relative
/// to `PlayerAbilities`.
#[test]
#[ignore = "daily"]
fn change_game_mode_updates_only_local_mode_and_abilities_26_3() {
    run_change_game_mode_updates_only_local_mode_and_abilities_26_3();
}

pub(crate) fn run_change_game_mode_updates_only_local_mode_and_abilities_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::change_game_mode_updates_only_local_mode_and_abilities_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["change_game_mode_abilities"]
                .as_array()
                .unwrap();

            // Case 0: Spectator (flying=false, custom speeds) -> Creative keeps flying=false and speeds.
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 3, None, [0.5, 64.0, 0.5]);
                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
                    .unwrap();
                game.handle_packet(abilities_packet(0x00, 0.125, 0.2))
                    .unwrap();
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                assert_row_core(game, &rows[0], false, None);
            });

            // Case 1: Spectator (flying=true, y=64.5 within 1 block of stone) -> Creative does NOT land.
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 3, None, [0.5, 64.5, 0.5]);
                game.handle_packet(abilities_packet(0x07, 0.125, 0.2))
                    .unwrap();
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                assert_row_core(game, &rows[1], false, None);
            });

            // Cases 2..=8: PlayerAbilities then CHANGE_GAME_MODE order, fractional rounding, invalid ids.
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 0, None, [0.5, 64.0, 0.5]);
                game.handle_packet(abilities_packet(0x0f, 0.08, 0.15))
                    .unwrap();
                game.handle_packet(change_mode_packet(0.0)).unwrap();
                assert_row_core(game, &rows[2], false, None);

                game.handle_packet(abilities_packet(0x0f, 0.08, 0.15))
                    .unwrap();
                game.handle_packet(change_mode_packet(2.0)).unwrap();
                assert_row_core(game, &rows[3], false, None);

                for (offset, param) in [(4, 0.6_f32), (5, 1.6), (6, 2.6), (7, 4.0), (8, -1.0)] {
                    game.handle_packet(change_mode_packet(param)).unwrap();
                    assert_row_core(game, &rows[offset], false, None);
                }
            });

            // Cases 9..=10: CHANGE_GAME_MODE then PlayerAbilities preserves mayBuild.
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 0, None, [0.5, 64.0, 0.5]);
                game.handle_packet(change_mode_packet(2.0)).unwrap();
                game.handle_packet(abilities_packet(0x0f, 0.07, 0.12))
                    .unwrap();
                assert_row_core(game, &rows[9], false, None);

                game.handle_packet(change_mode_packet(0.0)).unwrap();
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                assert_row_core(game, &rows[10], false, None);
            });

            // Case 11: repeating the same mode leaves previous_mode intact.
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 0, None, [0.5, 64.0, 0.5]);
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                assert_row_core(game, &rows[11], false, None);
            });
        },
    );
}

/// Real 26.3 oracle witness (`scenarios.split_mode_physics_matrix`, 48 rows):
/// `Player.tick()` (`noPhysics`, `onGround`, solid collision) follows `PlayerInfo`, while
/// `LocalPlayer.aiStep()` (forced spectator flight when `may_fly`, keeping `flying` on landing)
/// follows `MultiPlayerGameMode.isSpectator()`.
#[test]
#[ignore = "daily"]
fn split_local_and_player_info_physics_matrix_26_3() {
    run_split_local_and_player_info_physics_matrix_26_3();
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::abilities::gamemode_split_tests::run_split_local_and_player_info_physics_matrix_26_3;

/// Real 26.3 oracle witness (`scenarios.live_player_info_transitions`):
/// while `local_game_mode` stays Spectator and abilities stay 0x00, `PlayerInfoUpdate` flips
/// `noPhysics` live between `false` (absent / Adventure / Survival) and `true` (Spectator).
#[test]
#[ignore = "daily"]
fn live_player_info_updates_toggle_no_physics_while_local_spectator_26_3() {
    run_live_player_info_updates_toggle_no_physics_while_local_spectator_26_3();
}

pub(crate) fn run_live_player_info_updates_toggle_no_physics_while_local_spectator_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::live_player_info_updates_toggle_no_physics_while_local_spectator_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["live_player_info_transitions"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (mut peer, _writer) = setup_client(game, 3, None, [0.5, 64.0, 0.5]);
                game.handle_packet(abilities_packet(0x00, 0.0, 0.0))
                    .unwrap();
                let _ = drain_sent_abilities(game, &mut peer);

                for _ in 0..3 {
                    run_local_tick(game);
                }
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[0], true, Some(&sent));

                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
                    .unwrap();
                for _ in 0..3 {
                    run_local_tick(game);
                }
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[1], true, Some(&sent));

                game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
                for _ in 0..3 {
                    run_local_tick(game);
                }
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[2], true, Some(&sent));

                game.handle_packet(position_packet(2, [0.5, 64.0, 0.5]))
                    .unwrap();
                game.handle_packet(player_info_mode(SELF_UUID, 0)).unwrap();
                for _ in 0..3 {
                    run_local_tick(game);
                }
                let sent = drain_sent_abilities(game, &mut peer);
                assert_row_core(game, &rows[3], true, Some(&sent));
            });
        },
    );
}

/// Real 26.3 oracle witness (`scenarios.login_and_respawn_previous_mode`):
/// `Login` applies `spawn.previous_game_type`, and `Respawn` resets `Abilities` to defaults,
/// runs `adjustPlayer` with the previous local mode (`Spectator -> Creative` preserves `flying`),
/// then applies `setLocalMode(gameType, previousGameType)`.
#[test]
#[ignore = "daily"]
fn login_and_respawn_set_previous_mode_and_fresh_abilities_26_3() {
    run_login_and_respawn_set_previous_mode_and_fresh_abilities_26_3();
}

pub(crate) fn run_login_and_respawn_set_previous_mode_and_fresh_abilities_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::login_and_respawn_set_previous_mode_and_fresh_abilities_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["login_and_respawn_previous_mode"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 1, Some(2), [0.5, 64.0, 0.5]);
                assert_row_core(game, &rows[0], false, None);

                game.handle_packet(change_mode_packet(3.0)).unwrap();
                game.handle_packet(abilities_packet(0x07, 0.125, 0.2))
                    .unwrap();
                assert_row_core(game, &rows[1], false, None);

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn_info(1, Some(0)),
                    keep: 0,
                }))
                .unwrap();
                game.handle_packet(position_packet(2, [0.5, 64.0, 0.5]))
                    .unwrap();
                assert_row_core(game, &rows[2], false, None);

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn_info(2, None),
                    keep: 0,
                }))
                .unwrap();
                game.handle_packet(position_packet(3, [0.5, 64.0, 0.5]))
                    .unwrap();
                assert_row_core(game, &rows[3], false, None);
            });
        },
    );
}

/// Real 26.3 oracle witness (`scenarios.resolved_player_info_retention`):
/// `AbstractClientPlayer.getPlayerInfo` caches the first non-null `PlayerInfo` for the entity's
/// lifetime across `PlayerInfoRemove` and re-add as another mode, resolving afresh only on Respawn.
#[test]
#[ignore = "daily"]
fn resolved_player_info_retains_identity_across_remove_and_readd_26_3() {
    run_resolved_player_info_retains_identity_across_remove_and_readd_26_3();
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::abilities::gamemode_split_tests::run_resolved_player_info_retains_identity_across_remove_and_readd_26_3;

/// Real 26.3 oracle witness (`scenarios.respawn_keep_data_spectator_clears_y`):
/// `Respawn(KEEP_ENTITY_DATA)` followed by a Spectator `PlayerInfoUpdate` before the first
/// teleport clears the carried vertical velocity so a relative-delta teleport does not reuse it.
#[test]
#[ignore = "daily"]
fn respawn_keep_data_spectator_info_clears_pending_y_26_3() {
    run_respawn_keep_data_spectator_info_clears_pending_y_26_3();
}

pub(crate) fn run_respawn_keep_data_spectator_info_clears_pending_y_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::respawn_keep_data_spectator_info_clears_pending_y_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["respawn_keep_data_spectator_clears_y"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 0, None, [0.5, 64.0, 0.5]);
                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 0))
                    .unwrap();
                game.player.as_mut().unwrap().velocity = Vec3::new(0.25, 0.42, -0.1);

                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: spawn_info(0, None),
                    keep: 2,
                }))
                .unwrap();
                game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
                game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                    teleport: 2,
                    position: [0.5, 64.0, 0.5],
                    delta: [0.0, 0.0, 0.0],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: play::relative::DELTA_X
                        | play::relative::DELTA_Y
                        | play::relative::DELTA_Z,
                }))
                .unwrap();
                assert_row_core(game, &rows[1], true, None);
            });
        },
    );
}

/// Real 26.3 oracle witness (`scenarios.retained_player_login_keeps_speeds`):
/// a second `Login` while a logical player already exists retains custom fly and walk speeds and
/// the resolved `PlayerInfo` while running `adjustPlayer` + `setLocalMode`.
#[test]
#[ignore = "daily"]
fn retained_player_login_preserves_custom_ability_speeds_26_3() {
    run_retained_player_login_preserves_custom_ability_speeds_26_3();
}

pub(crate) fn run_retained_player_login_preserves_custom_ability_speeds_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::retained_player_login_preserves_custom_ability_speeds_26_3",
        || {
            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let rows = witness["scenarios"]["retained_player_login_keeps_speeds"]
                .as_array()
                .unwrap();

            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 1, None, [0.5, 64.0, 0.5]);
                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 3))
                    .unwrap();
                game.handle_packet(abilities_packet(0x0f, 0.125, 0.2))
                    .unwrap();
                assert_row_core(game, &rows[0], false, None);

                game.handle_packet(login_packet(83081, 0, Some(1))).unwrap();
                assert_row_core(game, &rows[1], false, None);

                game.handle_packet(login_packet(83081, 1, Some(0))).unwrap();
                assert_row_core(game, &rows[2], false, None);
            });
        },
    );
}

/// `ClientPacketListener.applyPlayerInfoUpdate`: runs `LocalPlayer.onGameModeChanged(SPECTATOR)`
/// before firing `ClientPlayerChangeGameTypeEvent` (which sees cleared Y motion and the old mode
/// before storing the new mode) and fires for any changed `PlayerInfo` profile without requiring
/// a spawned entity.
#[test]
#[ignore = "daily"]
fn player_info_hook_fires_after_motion_clear_and_for_tab_only_profiles() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::player_info_hook_fires_after_motion_clear_and_for_tab_only_profiles",
        || {
            use mod_api::{ClientHost, ClientPlayerChangeGameTypeEvent, Hooks, ModEntry};
            use std::sync::{Arc, Mutex};

            type Record = (Option<i32>, u128, i32, i32, Option<GameMode>, Option<f64>);
            struct Probe(Arc<Mutex<Vec<Record>>>);
            #[mod_api::__subscriptions]
            impl Hooks for Probe {
                fn client_player_change_game_type(
                    &self,
                    host: &mut dyn ClientHost,
                    event: &mut ClientPlayerChangeGameTypeEvent,
                ) {
                    let read: &dyn mod_api::Host = host;
                    let me = host.local_player();
                    let mode = me.and_then(|p| p.game_mode(read));
                    let vy = me.and_then(|p| p.entity().velocity(read)).map(|v| v[1]);
                    self.0.lock().unwrap().push((
                        event.player_id,
                        event.player_uuid,
                        event.current_game_type,
                        event.new_game_type,
                        mode,
                        vy,
                    ));
                }
            }

            let log = Arc::new(Mutex::new(Vec::new()));
            let mods: mod_api::Mods =
                vec![ModEntry::new("probe", Box::new(Probe(log.clone())))].into();
            mod_platform::testing::with_mods(mods, || {
                with_client_game(|game| {
                    let (_peer, _writer) = setup_client(game, 0, None, [0.5, 64.0, 0.5]);
                    game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 0))
                        .unwrap();
                    game.player.as_mut().unwrap().velocity = Vec3::new(0.2, 0.5, -0.3);
                    game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();

                    let other_uuid = 0xaaaa_bbbb_cccc_dddd_eeee_ffff_0000_1111_u128;
                    game.handle_packet(player_info_add_mode(other_uuid, "TabOnly", 0))
                        .unwrap();
                    game.handle_packet(player_info_mode(other_uuid, 2)).unwrap();

                    let entries = log.lock().unwrap().clone();
                    assert_eq!(entries.len(), 2);
                    assert_eq!(
                        entries[0],
                        (
                            Some(83081),
                            SELF_UUID,
                            0,
                            3,
                            Some(GameMode::Survival),
                            Some(0.0),
                        )
                    );
                    assert_eq!(
                        entries[1],
                        (None, other_uuid, 0, 2, Some(GameMode::Spectator), Some(0.0))
                    );
                    let me = game.local_player().unwrap();
                    assert_eq!(game.player_game_mode(me), Some(GameMode::Spectator));
                });
            });
        },
    );
}

/// Verifies HUD mode rules (#7), spectator prompt vs hotbar split (#8), entity-mode rendering
/// flags (#9), camera-entity fog split (#10), entity reach vs controller miss cooldown (#12),
/// tick-driven inventory conversion (#13), open World Options personal-mode sync (#16), and
/// per-binding synthetic middle-click dispatch (#18).
#[test]
#[ignore = "daily"]
fn split_mode_hud_rendering_reach_miss_inventory_and_input_rules() {
    run_split_mode_hud_rendering_reach_miss_inventory_and_input_rules();
}

pub(crate) fn run_split_mode_hud_rendering_reach_miss_inventory_and_input_rules() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::split_mode_hud_rendering_reach_miss_inventory_and_input_rules",
        || {
            use sunrise_client::fog::FogType;
            use sunrise_core::nbt::Tag;
            use sunrise_items::items::ItemStack;
            use sunrise_sim::motion::WATER;
            use winit::event::MouseButton;

            struct NoopLoop;
            impl sunrise_client_gameplay::game::event_loop::EventLoopCtl for NoopLoop {
                fn exit(&self) {}
                fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
            }

            let witness = oracle_witness();
            assert_eq!(witness["version"].as_str(), Some("26.3"));
            let water_rows = witness["scenarios"]["water_vision_tick"]
                .as_array()
                .unwrap();
            assert_eq!(water_rows.len(), 4);

            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 2, None, [0.5, 64.0, 0.5]);
                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
                    .unwrap();

                // #7: Adventure + instabuild=true still allows survival status / XP;
                // Creative + instabuild=false does not.
                game.handle_packet(abilities_packet(0x08, 0.05, 0.10))
                    .unwrap();
                assert!(game.instabuild());
                assert!(game.can_hurt_player());
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                assert!(!game.instabuild());
                assert!(!game.can_hurt_player());

                // #8 & #9: both directions of the local-controller vs PlayerInfo spectator split.
                game.handle_packet(change_mode_packet(3.0)).unwrap();
                assert!(game.spectator());
                assert!(!game.own_player_is_spectator());
                game.handle_packet(change_mode_packet(2.0)).unwrap();
                game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
                assert!(!game.spectator());
                assert!(game.own_player_is_spectator());

                // #10: camera_entity_is_spectator reads the attached camera entity's PlayerInfo.
                let other_uuid = 0x2222_3333_4444_5555_6666_7777_8888_9999_u128;
                let player_type = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:player")
                    .unwrap();
                game.handle_packet(player_info_add_mode(other_uuid, "CameraTarget", 0))
                    .unwrap();
                game.handle_packet(C::AddEntity(play::AddEntity {
                    id: 99,
                    uuid: other_uuid,
                    kind: player_type,
                    position: [2.5, 64.0, 2.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                }))
                .unwrap();
                game.client.remote.as_mut().unwrap().camera = Some(99);
                assert!(game.own_player_is_spectator());
                assert!(!game.camera_entity_is_spectator());
                game.handle_packet(player_info_mode(other_uuid, 3)).unwrap();
                assert!(game.camera_entity_is_spectator());
                game.client.remote.as_mut().unwrap().camera = None;

                // #10: underwater vision decides underwater from the local body's sampled eye fluid
                // (`Entity.baseTick` -> `EntityFluidInteraction.update` -> `LocalPlayer.aiStep`),
                // not camera fog (`camera_fog_type`).
                let water = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:water")
                    .unwrap();
                let air = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .unwrap();
                game.client.fog.water_vision_time = 0;
                game.handle_packet(abilities_packet(0x06, 0.05, 0.10))
                    .unwrap();
                for y in 64..=66 {
                    game.world.set(0, y, 0, water).unwrap();
                }
                assert_eq!(
                    game.camera_fog_type(game.eye_position().unwrap()),
                    FogType::Water
                );
                assert!(!game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                // Before the 1-block neighbor chunks around `(0.5, 64.0, 0.5)` are loaded,
                // `EntityFluidInteraction.hasFluidAndLoaded` skips sampling so `isEyeInFluid`
                // stays false even while `camera_fog_type` is Water.
                game.tick_remote_level().unwrap();
                assert!(!game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                assert_eq!(game.client.fog.water_vision_time, 0);

                // Load the neighbor chunks required by `EntityFluidInteraction.hasFluidAndLoaded`
                // (`x0 - 1 ..= x1 + 1`, `z0 - 1 ..= z1 + 1`), then attach the camera to entity 99
                // in air while the local body is in water.
                for (cx, cz) in [(-1, -1), (-1, 0), (0, -1)] {
                    game.client
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(cx, cz));
                }
                game.client.remote.as_mut().unwrap().camera = Some(99);
                assert_eq!(
                    game.camera_fog_type(game.aim().unwrap().0),
                    FogType::Atmospheric
                );
                game.handle_packet(change_mode_packet(3.0)).unwrap();
                game.handle_packet(player_info_mode(SELF_UUID, 2)).unwrap();
                game.tick_remote_level().unwrap();
                assert!(game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                assert_eq!(
                    game.client.fog.water_vision_time,
                    water_rows[0]["water_vision_time"].as_i64().unwrap() as i32
                );
                game.handle_packet(change_mode_packet(2.0)).unwrap();
                game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
                game.tick_remote_level().unwrap();
                assert!(game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                assert_eq!(
                    game.client.fog.water_vision_time,
                    water_rows[1]["water_vision_time"].as_i64().unwrap() as i32
                );

                // Clear water at the local body while placing water at camera entity 99 so camera
                // fog is Water while the local body's sampled eye fluid is dry.
                for y in 64..=66 {
                    game.world.set(0, y, 0, air).unwrap();
                    game.world.set(2, y, 2, water).unwrap();
                }
                assert_eq!(game.camera_fog_type(game.aim().unwrap().0), FogType::Water);
                game.tick_remote_level().unwrap();
                assert!(!game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                assert_eq!(
                    game.client.fog.water_vision_time,
                    water_rows[2]["water_vision_time"].as_i64().unwrap() as i32
                );
                game.handle_packet(change_mode_packet(3.0)).unwrap();
                game.tick_remote_level().unwrap();
                assert!(!game.player.as_ref().unwrap().is_eye_in_fluid(WATER));
                assert_eq!(
                    game.client.fog.water_vision_time,
                    water_rows[3]["water_vision_time"].as_i64().unwrap() as i32
                );
                game.client.remote.as_mut().unwrap().camera = None;
                for y in 64..=66 {
                    game.world.set(0, y, 0, air).unwrap();
                    game.world.set(2, y, 2, air).unwrap();
                }
                if let Some(player) = game.player.as_mut() {
                    player.position = Vec3::new(0.5, 64.0, 0.5);
                    player.velocity = Vec3::ZERO;
                }

                // #12: attack_range uses entity isCreative; miss cooldown uses controller hasMissTime.
                let mut spear = ItemStack::new("minecraft:stick", 1);
                spear.data_mut().set_other(
                    "minecraft:attack_range",
                    Some(Tag::Compound(
                        [
                            ("min_reach".into(), Tag::Float(0.0_f32.to_bits())),
                            ("max_reach".into(), Tag::Float(3.0_f32.to_bits())),
                            ("min_creative_reach".into(), Tag::Float(0.0_f32.to_bits())),
                            ("max_creative_reach".into(), Tag::Float(5.0_f32.to_bits())),
                            ("hitbox_margin".into(), Tag::Float(0.0_f32.to_bits())),
                        ]
                        .into_iter()
                        .collect(),
                    )),
                );
                game.selected = 0;
                game.inventory[0] = Some(spear);
                game.handle_packet(player_info_mode(SELF_UUID, 2)).unwrap();
                game.handle_packet(abilities_packet(0x08, 0.05, 0.10))
                    .unwrap();
                assert!(!game.own_player_is_creative());
                assert!(!game.held_range_allows(4.0));
                game.handle_packet(player_info_mode(SELF_UUID, 1)).unwrap();
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                assert!(game.own_player_is_creative());
                assert!(game.held_range_allows(4.0));

                game.handle_packet(change_mode_packet(0.0)).unwrap();
                game.handle_packet(abilities_packet(0x08, 0.05, 0.10))
                    .unwrap();
                game.set_test_forced_hit(None);
                game.clicks.miss_time = 0;
                assert!(game.has_miss_time());
                game.remote_start_attack();
                assert_eq!(game.clicks.miss_time, 10);
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                game.clicks.miss_time = 0;
                assert!(!game.has_miss_time());
                game.remote_start_attack();
                assert_eq!(game.clicks.miss_time, 0);

                // #13: server-opened container menus stay open even with instabuild=true, while
                // the two player-inventory screens reconcile on tick and preserve the carried stack.
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                game.handle_packet(abilities_packet(0x08, 0.05, 0.10))
                    .unwrap();
                let chest_menu = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:menu")
                    .unwrap()
                    .id("minecraft:generic_9x3")
                    .unwrap();
                game.handle_packet(C::OpenScreen(play::OpenScreen {
                    container: 7,
                    menu: chest_menu,
                    title: Tag::String("Chest".into()),
                }))
                .unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.remote_menu_type(), Some("minecraft:generic_9x3"));
                assert!(!game.is_player_inventory_screen());
                game.tick_remote_level().unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.remote_menu_type(), Some("minecraft:generic_9x3"));
                assert!(game.container.is_some());

                let crafting_menu = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:menu")
                    .unwrap()
                    .id("minecraft:crafting")
                    .unwrap();
                game.handle_packet(C::OpenScreen(play::OpenScreen {
                    container: 8,
                    menu: crafting_menu,
                    title: Tag::String("Crafting".into()),
                }))
                .unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert!(!game.is_player_inventory_screen());
                game.tick_remote_level().unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.remote_menu_type(), Some("minecraft:crafting"));
                assert_eq!(game.craft_size, 3);

                game.remote_close_local();
                game.set_screen(Screen::Inventory);
                assert!(game.is_player_inventory_screen());
                game.carried = Some(ItemStack::new("minecraft:diamond", 5));
                game.handle_packet(change_mode_packet(1.0)).unwrap();
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                game.tick_remote_level().unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 5))
                );
                game.handle_packet(abilities_packet(0x08, 0.05, 0.10))
                    .unwrap();
                game.tick_remote_level().unwrap();
                assert_eq!(game.client.screen, Screen::Creative);
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 5))
                );
                game.handle_packet(abilities_packet(0x00, 0.05, 0.10))
                    .unwrap();
                game.tick_remote_level().unwrap();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(
                    game.carried.as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:diamond", 5))
                );
                game.carried = None;

                // #16: external set_local_mode refreshes open WorldOptions personal mode + baseline
                // without discarding unrelated edits.
                game.set_local_mode(GameMode::Survival);
                game.open_world_options();
                game.set_world_options_difficulty_for_test(3);
                game.set_local_mode(GameMode::Creative);
                assert_eq!(game.world_options_personal_state(), Some((1, 1, 3)));
                game.close_world_options();

                // #18: synthetic button and key edges route through per-binding dispatch once per
                // binding, even when movement, toggle, and plain actions share a rebound input.
                let (reply, _rx) = std::sync::mpsc::channel();
                game.set_screen(Screen::Playing);
                game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
                assert!(game.own_player_is_spectator());
                let pick_before = game.pending_mapping_clicks("key.pickItem");
                let spec_before = game.pending_mapping_clicks("key.spectatorHotbar");
                game.play_mouse(MouseButton::Middle, true).unwrap();
                game.play_mouse(MouseButton::Middle, false).unwrap();
                assert_eq!(game.pending_mapping_clicks("key.pickItem"), pick_before + 1);
                assert_eq!(
                    game.pending_mapping_clicks("key.spectatorHotbar"),
                    spec_before + 1
                );
                game.control_command("mouse middle click", &reply, &NoopLoop)
                    .unwrap();
                assert_eq!(game.pending_mapping_clicks("key.pickItem"), pick_before + 2);
                assert_eq!(
                    game.pending_mapping_clicks("key.spectatorHotbar"),
                    spec_before + 2
                );

                // Rebind movement (key.forward), keyboard toggle (key.sneak), mouse toggle
                // (key.attack), and plain action (key.drop) onto key.mouse.middle alongside the
                // default middle pair: every binding triggers exactly once.
                game.client.settings.toggle_crouch = true;
                game.client
                    .settings
                    .keys
                    .set("key.forward", "key.mouse.middle");
                game.client
                    .settings
                    .keys
                    .set("key.sneak", "key.mouse.middle");
                game.client
                    .settings
                    .keys
                    .set("key.attack", "key.mouse.middle");
                game.client
                    .settings
                    .keys
                    .set("key.drop", "key.mouse.middle");
                let before = [
                    game.pending_mapping_clicks("key.forward"),
                    game.pending_mapping_clicks("key.sneak"),
                    game.pending_mapping_clicks("key.attack"),
                    game.pending_mapping_clicks("key.drop"),
                    game.pending_mapping_clicks("key.pickItem"),
                    game.pending_mapping_clicks("key.spectatorHotbar"),
                ];
                game.control_command("mouse middle down", &reply, &NoopLoop)
                    .unwrap();
                assert!(game.mapped_key_down("key.forward"));
                assert!(game.mapped_key_down("key.sneak"));
                assert!(game.mapped_key_down("key.attack"));
                assert!(game.mapped_key_down("key.drop"));
                for (name, prev) in [
                    ("key.forward", before[0]),
                    ("key.sneak", before[1]),
                    ("key.attack", before[2]),
                    ("key.drop", before[3]),
                    ("key.pickItem", before[4]),
                    ("key.spectatorHotbar", before[5]),
                ] {
                    assert_eq!(
                        game.pending_mapping_clicks(name),
                        prev + 1,
                        "{name} after mouse middle down"
                    );
                }
                game.control_command("mouse middle up", &reply, &NoopLoop)
                    .unwrap();
                assert!(!game.mapped_key_down("key.forward"));
                assert!(game.mapped_key_down("key.sneak"));
                assert!(!game.mapped_key_down("key.attack"));
                assert!(!game.mapped_key_down("key.drop"));
                game.control_command("mouse middle click", &reply, &NoopLoop)
                    .unwrap();
                assert!(!game.mapped_key_down("key.sneak"));

                // Rebinding key.mouse.left away from key.attack to key.forward dispatches only
                // key.forward and not key.attack.
                game.client.settings.keys.reset_all();
                game.client.settings.toggle_crouch = false;
                game.client
                    .settings
                    .keys
                    .set("key.attack", "key.keyboard.r");
                game.client
                    .settings
                    .keys
                    .set("key.forward", "key.mouse.left");
                let fwd_before = game.pending_mapping_clicks("key.forward");
                let atk_before = game.pending_mapping_clicks("key.attack");
                game.control_command("mouse left click", &reply, &NoopLoop)
                    .unwrap();
                assert_eq!(game.pending_mapping_clicks("key.forward"), fwd_before + 1);
                assert_eq!(game.pending_mapping_clicks("key.attack"), atk_before);

                // Rebinding key.pickItem, key.drop, key.forward, key.sneak, and key.attack onto
                // one keyboard key dispatches each binding exactly once (never double-queueing
                // key.drop or skipping movement/toggle actions).
                game.client.settings.keys.reset_all();
                game.client
                    .settings
                    .keys
                    .set("key.pickItem", "key.keyboard.g");
                game.client.settings.keys.set("key.drop", "key.keyboard.g");
                game.client
                    .settings
                    .keys
                    .set("key.forward", "key.keyboard.g");
                game.client.settings.keys.set("key.sneak", "key.keyboard.g");
                game.client
                    .settings
                    .keys
                    .set("key.attack", "key.keyboard.g");
                let key_before = [
                    game.pending_mapping_clicks("key.pickItem"),
                    game.pending_mapping_clicks("key.drop"),
                    game.pending_mapping_clicks("key.forward"),
                    game.pending_mapping_clicks("key.sneak"),
                    game.pending_mapping_clicks("key.attack"),
                ];
                game.control_command("key g tap", &reply, &NoopLoop)
                    .unwrap();
                for (name, prev) in [
                    ("key.pickItem", key_before[0]),
                    ("key.drop", key_before[1]),
                    ("key.forward", key_before[2]),
                    ("key.sneak", key_before[3]),
                    ("key.attack", key_before[4]),
                ] {
                    assert_eq!(
                        game.pending_mapping_clicks(name),
                        prev + 1,
                        "{name} after key g tap"
                    );
                }
                game.client.settings.keys.reset_all();
            });
        },
    );
}

pub(crate) fn add_entity_packet(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    kind_name: &str,
) -> C {
    let kind = game
        .client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind_name)
        .unwrap();
    C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind,
        position: [2.5, 64.0, 2.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    })
}

pub(crate) fn spawn_mob(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    kind: &str,
    max_health: f32,
    health: f32,
    saddled: bool,
) {
    let pkt = add_entity_packet(game, id, uuid, kind);
    game.handle_packet(pkt).unwrap();
    // The HUD reads the tracked entity's synced max health, not the mob's local cache.
    let attr = game.motion.attributes.id("minecraft:max_health").unwrap();
    game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
        id,
        values: vec![play::AttributeSnapshot {
            attribute: attr.registry_id(),
            base: f64::from(max_health),
            modifiers: vec![],
        }],
    }))
    .unwrap();
    let mob = game
        .mobs
        .iter_mut()
        .find(|m| m.id == id as u32)
        .expect("spawned mob");
    mob.health = health;
    mob.extra.set_saddled(saddled);
}

pub(crate) fn contextual_bar_name(bar: sunrise_client::locator_bar::ContextualBar) -> &'static str {
    use sunrise_client::locator_bar::ContextualBar;
    match bar {
        ContextualBar::Empty => "empty",
        ContextualBar::Experience => "experience",
        ContextualBar::Locator => "locator",
        ContextualBar::JumpableVehicle => "jumpable_vehicle",
    }
}

/// Real 26.3 oracle (`scenarios.hud_camera_mounts`): vehicle hearts follow `getCameraPlayer`, jump bar follows `LocalPlayer`.
#[test]
#[ignore = "daily"]
fn hud_camera_mounts_match_26_3_oracle() {
    run_hud_camera_mounts_match_26_3_oracle();
}

pub(crate) fn run_hud_camera_mounts_match_26_3_oracle() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::abilities::gamemode_split_tests::hud_camera_mounts_match_26_3_oracle",
        || {
            let witness = oracle_witness();
            let rows = witness["scenarios"]["hud_camera_mounts"]
                .as_array()
                .expect("hud_camera_mounts rows");
            with_client_game(|game| {
                let (_peer, _writer) = setup_client(game, 3, None, [0.5, 64.0, 0.5]);
                game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
                    .unwrap();

                spawn_mob(game, 201, 0x1001, "minecraft:horse", 30.0, 24.0, true);
                let stand_pkt = add_entity_packet(game, 202, 0x1002, "minecraft:armor_stand");
                game.handle_packet(stand_pkt).unwrap();
                spawn_mob(game, 203, 0x1003, "minecraft:pig", 10.0, 10.0, false);

                for (id, uuid, name) in [
                    (204, 0x1004_u128, "RemoteUnmounted"),
                    (205, 0x1005_u128, "RemoteOnPig"),
                    (207, 0x1007_u128, "RemoteOnHorse"),
                ] {
                    game.handle_packet(player_info_add_mode(uuid, name, 0))
                        .unwrap();
                    let pkt = add_entity_packet(game, id, uuid, "minecraft:player");
                    game.handle_packet(pkt).unwrap();
                }
                spawn_mob(game, 206, 0x1006, "minecraft:pig", 10.0, 8.0, false);
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: 206,
                    passengers: vec![205],
                }))
                .unwrap();
                spawn_mob(game, 208, 0x1008, "minecraft:horse", 30.0, 20.0, true);
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: 208,
                    passengers: vec![207],
                }))
                .unwrap();

                let cases = [
                    ("spectator_local_mounted_camera_self", 3.0_f32, true, 83081),
                    ("spectator_local_mounted_camera_armor_stand", 3.0, true, 202),
                    ("spectator_local_mounted_camera_pig", 3.0, true, 203),
                    ("survival_local_mounted_camera_armor_stand", 0.0, true, 202),
                    (
                        "spectator_local_mounted_camera_remote_unmounted",
                        3.0,
                        true,
                        204,
                    ),
                    (
                        "spectator_local_mounted_camera_remote_mounted_pig",
                        3.0,
                        true,
                        205,
                    ),
                    (
                        "spectator_local_unmounted_camera_remote_mounted_horse",
                        3.0,
                        false,
                        207,
                    ),
                ];
                assert_eq!(rows.len(), cases.len());
                for (row, &(expected_step, mode_param, local_mounted, camera_id)) in
                    rows.iter().zip(cases.iter())
                {
                    let label = row["step"].as_str().unwrap();
                    assert_eq!(label, expected_step);
                    game.handle_packet(change_mode_packet(mode_param)).unwrap();
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle: 201,
                        passengers: if local_mounted { vec![83081] } else { vec![] },
                    }))
                    .unwrap();
                    game.handle_packet(C::SetCamera(play::SetCamera { camera_id }))
                        .unwrap();

                    assert_eq!(
                        game.mode.name(),
                        row["local_game_mode"].as_str().unwrap(),
                        "{label}: local_game_mode"
                    );
                    let camera = game.camera_player();
                    assert_eq!(
                        camera
                            != sunrise_client_gameplay::game::remote_camera::CameraPlayer::Absent,
                        row["camera_is_player"].as_bool().unwrap(),
                        "{label}: camera_is_player"
                    );
                    let got_vehicle = game.camera_vehicle_health(camera);
                    assert_eq!(
                        got_vehicle.map_or(0, |(hearts, _)| hearts),
                        row["vehicle_max_hearts"].as_i64().unwrap() as i32,
                        "{label}: vehicle_max_hearts"
                    );
                    let want_health_bits = (!row["vehicle_health_bits"].is_null())
                        .then(|| f32_bits(&row["vehicle_health_bits"]));
                    assert_eq!(
                        got_vehicle.map(|(_, health)| health.to_bits()),
                        want_health_bits,
                        "{label}: vehicle_health_bits"
                    );
                    assert_eq!(
                        contextual_bar_name(game.contextual_bar()),
                        row["contextual_info"].as_str().unwrap(),
                        "{label}: contextual_info"
                    );
                }
            });
        },
    );
}

/// `EntityRef::spectator` on a client (#5613): the local player and a tracked player answer from
/// their `PlayerInfo` game mode, as `AbstractClientPlayer.gameMode` does, and a mob is no
/// spectator.
#[test]
#[ignore = "daily"]
fn a_client_reads_each_players_spectator_mode_from_its_player_info() {
    with_client_game(|game| {
        let (_peer, _writer) = setup_client(game, 2, None, [0.5, 64.0, 0.5]);
        game.handle_packet(player_info_add_mode(SELF_UUID, SELF_NAME, 2))
            .unwrap();
        let me = game.local_player().unwrap().entity();
        assert_eq!(me.spectator(game), Some(false));
        game.handle_packet(player_info_mode(SELF_UUID, 3)).unwrap();
        assert_eq!(me.spectator(game), Some(true));

        let builtin = &game.client.remote.as_ref().unwrap().builtin;
        let types = builtin.get("minecraft:entity_type").unwrap();
        let (player_type, zombie_type) = (
            types.id("minecraft:player").unwrap(),
            types.id("minecraft:zombie").unwrap(),
        );
        let other_uuid = 0x2222_3333_4444_5555_6666_7777_8888_9999_u128;
        game.handle_packet(player_info_add_mode(other_uuid, "Watcher", 0))
            .unwrap();
        for (id, uuid, kind) in [(99, other_uuid, player_type), (100, 100, zombie_type)] {
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid,
                kind,
                position: [2.5, 64.0, 2.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
        }
        let other = game.entity(99).unwrap();
        let zombie = game.entity(100).unwrap();
        assert_eq!(other.spectator(game), Some(false));
        game.handle_packet(player_info_mode(other_uuid, 3)).unwrap();
        assert_eq!(other.spectator(game), Some(true));
        assert_eq!(zombie.spectator(game), Some(false));
    });
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
