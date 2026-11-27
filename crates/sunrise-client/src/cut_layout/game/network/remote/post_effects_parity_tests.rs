//! Play-phase `post_effects` lifecycle and pre-login protocol error against 26.3.
#![cfg(feature = "client")]
use std::time::{Duration, Instant};
use sunrise_protocol::{
    connection::Connection,
    packets::{
        configuration as cfg,
        play::{self, Clientbound as C, Serverbound as S, SpawnInfo},
    },
};

pub(crate) fn post_effects_witness() -> serde_json::Value {
    let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR")
        .expect("SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)");
    let path = std::path::Path::new(&dir).join("post_effects.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn overworld_spawn() -> SpawnInfo {
    SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

pub(crate) fn login_packet(player_id: i32) -> play::Login {
    play::Login {
        player_id,
        hardcore: false,
        levels: vec!["minecraft:overworld".into()],
        max_players: 20,
        chunk_radius: 10,
        simulation_distance: 10,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: overworld_spawn(),
        online_mode: false,
        enforces_secure_chat: false,
    }
}

pub(crate) fn json_strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

pub(crate) fn wait_for_pong(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    peer: &mut Connection,
    ping_id: i32,
) {
    peer.send(&C::Ping(cfg::Ping { id: ping_id }).encode())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        game.poll_remote().unwrap();
        while let Ok((id, body)) = peer.recv() {
            if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == ping_id) {
                return;
            }
        }
        std::thread::yield_now();
    }
    panic!("timed out waiting for Pong({ping_id})");
}

#[test]
#[ignore = "daily"]
fn post_effects_cleared_on_login_respawn_and_reconfigure_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::ingress_state::post_effects_parity_tests::post_effects_cleared_on_login_respawn_and_reconfigure_26_3",
        post_effects_cleared_on_login_respawn_and_reconfigure_body,
    );
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::raw_ingress::post_effects_parity_tests::post_effects_cleared_on_login_respawn_and_reconfigure_body;

#[test]
#[ignore = "daily"]
fn post_effects_before_login_disconnects_with_packet_error_26_3() {
    const NAME: &str = "cut_layout::game::client_side::remote_client::ingress_state::post_effects_parity_tests::post_effects_before_login_disconnects_with_packet_error_26_3";
    const FLAG: &str = "SUNRISE_POST_EFFECTS_BEFORE_LOGIN_CASE";
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &["empty", "nonempty"],
    ) {
        return;
    }
    let case_name = std::env::var(FLAG).unwrap();
    post_effects_before_login_disconnects_case(&case_name);
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::raw_ingress::post_effects_parity_tests::post_effects_before_login_disconnects_case;
