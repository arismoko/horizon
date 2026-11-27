//! Atomic play packet bundle dispatch against the recorded 26.3 witness.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use std::time::{Duration, Instant};
use sunrise_client::net::Session;
use sunrise_protocol::{
    connection::Pipe,
    packets::{
        Empty, configuration as cfg,
        play::{self, Clientbound as C, Serverbound as S},
    },
};

pub(crate) fn witness_json(name: &str) -> serde_json::Value {
    let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR")
        .expect("SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)");
    let path = std::path::Path::new(&dir).join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn bundle_witness() -> serde_json::Value {
    witness_json("bundle.json")
}

pub(crate) fn format_serverbound(packet: S) -> Option<String> {
    match packet {
        S::Pong(p) => Some(format!("pong:{}", p.id)),
        S::KeepAlive(k) => Some(format!("keep_alive:{}", k.id)),
        S::MovePlayerRot(m) => Some(format!(
            "move_player_rot:{:08x}:{:08x}",
            m.yaw.to_bits(),
            m.pitch.to_bits()
        )),
        _ => None,
    }
}

pub(crate) fn json_strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_owned())
        .collect()
}

#[test]
#[ignore = "daily"]
fn genpvp_ping_rotation_and_keepalive_bundle_apply_atomically_in_order_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::ingress_state::bundle_parity_tests::genpvp_ping_rotation_and_keepalive_bundle_apply_atomically_in_order_26_3",
        genpvp_ping_rotation_and_keepalive_bundle_body,
    );
}

pub(crate) fn genpvp_ping_rotation_and_keepalive_bundle_body() {
    let witness = bundle_witness();
    let ping_rot = &witness["genpvp_ping_rotation_dispatch"];
    let expected_ping_rot = json_strings(&ping_rot["sent_after_process"]);
    let expected_yaw_bits =
        u32::from_str_radix(ping_rot["player_yRot_bits"].as_str().unwrap(), 16).unwrap();
    let expected_pitch_bits =
        u32::from_str_radix(ping_rot["player_xRot_bits"].as_str().unwrap(), 16).unwrap();
    let expected_keepalive =
        json_strings(&witness["keepalive_ordering"]["bundled"]["sent_after_process"]);

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "BundleOrder").unwrap();
        let (session, mut peer) = Session::play_loop_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        remote.logged_in = true;
        remote.entity_id = 83081;
        game.yaw = 0.0;
        game.pitch = 0.0;
        peer.reader
            .set_read_timeout(Some(Duration::from_millis(40)))
            .unwrap();

        // Open bundle without closing delimiter: neither packet may apply yet.
        for packet in [
            C::BundleDelimiter(Empty),
            C::Ping(cfg::Ping { id: -26 }),
            C::PlayerRotation(play::PlayerRotation {
                yaw: 22.5,
                relative_yaw: false,
                pitch: -11.25,
                relative_pitch: false,
            }),
        ] {
            peer.send(&packet.encode()).unwrap();
        }
        let mut early = Vec::new();
        let hold_until = Instant::now() + Duration::from_millis(80);
        while Instant::now() < hold_until {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if let Some(tag) = format_serverbound(S::decode(id, &body).unwrap()) {
                    early.push(tag);
                }
            }
            std::thread::yield_now();
        }
        assert!(early.is_empty(), "unclosed bundle sent replies: {early:?}");
        assert_eq!(
            game.yaw.to_bits(),
            0.0f32.to_bits(),
            "unclosed bundle mutated player yaw"
        );

        // Close bundle: both packets apply together in wire order.
        peer.send(&C::BundleDelimiter(Empty).encode()).unwrap();
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.len() < expected_ping_rot.len() && Instant::now() < deadline {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if let Some(tag) = format_serverbound(S::decode(id, &body).unwrap()) {
                    seen.push(tag);
                }
            }
            std::thread::yield_now();
        }
        assert_eq!(seen, expected_ping_rot);
        assert_eq!(game.yaw.to_bits(), expected_yaw_bits);
        assert_eq!(game.pitch.to_bits(), expected_pitch_bits);

        // Bundled KeepAlive stays in bundle order behind Ping and ahead of PlayerRotation.
        for packet in [
            C::BundleDelimiter(Empty),
            C::Ping(cfg::Ping { id: 7 }),
            C::KeepAlive(cfg::KeepAlive { id: 999 }),
            C::PlayerRotation(play::PlayerRotation {
                yaw: 90.0,
                relative_yaw: false,
                pitch: 12.5,
                relative_pitch: false,
            }),
            C::BundleDelimiter(Empty),
        ] {
            peer.send(&packet.encode()).unwrap();
        }
        assert!(
            peer.recv().is_err(),
            "bundled KeepAlive answered before game thread polled"
        );
        let mut keepalive_seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while keepalive_seen.len() < expected_keepalive.len() && Instant::now() < deadline {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if let Some(tag) = format_serverbound(S::decode(id, &body).unwrap()) {
                    keepalive_seen.push(tag);
                }
            }
            std::thread::yield_now();
        }
        assert_eq!(keepalive_seen, expected_keepalive);
    });
}

#[test]
#[ignore = "daily"]
fn dependent_entity_bundle_never_splits_across_poll_or_tick_26_3() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::ingress_state::bundle_parity_tests::dependent_entity_bundle_never_splits_across_poll_or_tick_26_3",
        dependent_entity_bundle_body,
    );
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::raw_ingress::bundle_parity_tests::dependent_entity_bundle_body;

#[test]
#[ignore = "daily"]
fn entity_data_validation_inline_painting_and_pre_login_bundle_errors_26_3() {
    const NAME: &str = "cut_layout::game::client_side::remote_client::ingress_state::bundle_parity_tests::entity_data_validation_inline_painting_and_pre_login_bundle_errors_26_3";
    const FLAG: &str = "SUNRISE_BUNDLE_PARITY_ENTITY_DATA_CASE";
    const CASES: &[&str] = &[
        "before_login_metadata",
        "metadata_validation_and_inline_painting",
        "synced_painting_override",
        "pre_login_disconnect_bundle",
        "pre_login_post_effects_bundle",
        "disconnect_notify_after_shutdown",
    ];
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(NAME, FLAG, CASES) {
        return;
    }
    let case_name = std::env::var(FLAG).unwrap();
    entity_data_validation_inline_painting_and_pre_login_bundle_case(&case_name);
}

pub(crate) fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

pub(crate) fn wait_pong_in_game(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    peer: &mut sunrise_protocol::connection::Connection,
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

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::raw_ingress::bundle_parity_tests::entity_data_validation_inline_painting_and_pre_login_bundle_case;
