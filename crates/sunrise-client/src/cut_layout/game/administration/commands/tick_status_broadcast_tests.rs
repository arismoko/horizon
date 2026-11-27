//! `/tick`'s status lines reach the other operators; its query lines stay
//! private. `TickCommand.setFreeze` and `sprint` call
//! `sendSuccess(..., true)` (broadcast) while `tickQuery` passes `false`.
use serde_json::json;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Dim;
use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use sunrise_sim::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};

/// The chat components a guest's connection was sent since the last flush.
fn chats(receiver: &mpsc::Receiver<sunrise_sim::lan::Out>) -> Vec<serde_json::Value> {
    let mut result = Vec::new();
    for sent in receiver.try_iter() {
        let bodies = match sent {
            sunrise_sim::lan::Out::Packet(p) => vec![p],
            sunrise_sim::lan::Out::Packets(p) => p,
            sunrise_sim::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Clientbound::SystemChat(p) = Clientbound::decode(id, reader.rest()).unwrap() {
                result.push(sunrise_game_data::scoreboard::tag_to_json(&p.content));
            }
        }
    }
    result
}

/// The host's queued `broadcastToAdmins` lines, sent as `tick_lan` does for
/// the host's own commands, then what the operator received.
fn flush_to_operator(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    receiver: &mpsc::Receiver<sunrise_sim::lan::Out>,
) -> Vec<serde_json::Value> {
    game.send_admin_feedback();
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    chats(receiver)
}

/// The `[Player: <message>]` line `broadcastToAdmins` builds, gray and italic.
fn admin_line(message: serde_json::Value) -> serde_json::Value {
    json!({
        "translate": "chat.type.admin",
        "with": [{"text": "Player"}, message],
        "color": "gray",
        "italic": true,
    })
}

/// A LAN world the host opened, with one other operator (conn 7) connected.
fn lan_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (
    sunrise_gameplay::game::ServerGame<'a>,
    mpsc::Receiver<sunrise_sim::lan::Out>,
) {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 66.0, 0.5)).unwrap());
    game.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "tick-status".into(), 4)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 4,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 7,
            uuid: 7,
            name: "Operator".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        7,
        [8.5, 66.0, 8.5],
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.permission = 2;
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    // An operator (`PlayerList.isOp`): listed in ops.json.
    hub.ops.insert(7, 2);
    hub.guests.push(guest);
    (game, rx)
}

#[test]
#[ignore = "daily"]
fn tick_status_lines_reach_the_other_operators() {
    let mut resources = None;
    let (mut game, rx) = lan_game(&mut resources);
    assert!(game.run_command("tick freeze"));
    assert_eq!(
        flush_to_operator(&mut game, &rx),
        vec![admin_line(
            json!({"translate": "commands.tick.status.frozen"})
        )]
    );
    assert!(game.run_command("tick unfreeze"));
    assert_eq!(
        flush_to_operator(&mut game, &rx),
        vec![admin_line(json!({
            "translate": "commands.tick.status.running"
        }))]
    );
    assert!(game.run_command("tick sprint 20"));
    assert_eq!(
        flush_to_operator(&mut game, &rx),
        vec![admin_line(json!({
            "translate": "commands.tick.status.sprinting"
        }))]
    );
    // `tickQuery` passes `broadcast` false: nothing reaches the operator.
    assert!(game.run_command("tick query"));
    assert_eq!(
        flush_to_operator(&mut game, &rx),
        Vec::<serde_json::Value>::new()
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
