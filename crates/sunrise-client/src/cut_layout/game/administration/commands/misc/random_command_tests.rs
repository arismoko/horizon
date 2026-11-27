//! `/random`'s roll line: who hears it and who it names, and the names
//! `/random reset` completes (RandomCommand).
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use {sunrise_gameplay::game::game_tests::tests::server_game, sunrise_gameplay_common::game::Dim};

/// The packets a guest's connection was sent, encoded.
pub(crate) fn packets(receiver: &mpsc::Receiver<Out>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for sent in receiver.try_iter() {
        match sent {
            Out::Packet(p) => out.push(p),
            Out::Packets(p) => out.extend(p),
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        }
    }
    out
}

/// The system lines a guest's connection was sent (`ClientboundSystemChatPacket`).
pub(crate) fn chats(receiver: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut out = Vec::new();
    for body in packets(receiver) {
        let mut reader = sunrise_protocol::buf::Reader::new(&body);
        let id = reader.varint().unwrap();
        if let Clientbound::SystemChat(p) = Clientbound::decode(id, reader.rest()).unwrap() {
            // The client's own read of the line it is sent.
            out.push(
                sunrise_game_data::scoreboard::comp_text(
                    &sunrise_game_data::scoreboard::tag_to_json(&p.content),
                )
                .plain(),
            );
        }
    }
    out
}

/// The completions a guest was sent (`ClientboundCommandSuggestionsPacket`).
pub(crate) fn suggestions(receiver: &mpsc::Receiver<Out>) -> Vec<String> {
    let mut out = Vec::new();
    for body in packets(receiver) {
        let mut r = sunrise_protocol::buf::Reader::new(&body);
        if r.varint().ok() != Some(sunrise_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS) {
            continue;
        }
        let _ = (r.varint(), r.varint(), r.varint());
        for _ in 0..r.count(usize::MAX).unwrap_or(0) {
            out.extend(r.string(32767).ok());
            if r.bool().unwrap_or(false) {
                let _ = r.nbt();
            }
        }
    }
    out
}

/// A server with two guests on it (`conn` 2 and 7).
pub(crate) fn server_with_guests<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (
    sunrise_gameplay::game::ServerGame<'a>,
    Vec<mpsc::Receiver<Out>>,
) {
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "random-roll".into(), 4)
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
    let mut receivers = Vec::new();
    for conn in [2, 7] {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("Roller{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            (1_000 + conn) as i32,
            [8.5, 200.0, 8.5],
            2,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.permission = 2;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        receivers.push(rx);
    }
    (game, receivers)
}

/// PlayerList.broadcastSystemMessage: a guest's roll reaches every player,
/// the rolling guest once (its copy is the command's captured output).
#[test]
#[ignore = "daily"]
fn a_guests_roll_reaches_every_player_once() {
    let mut resources = None;
    let (mut game, receivers) = server_with_guests(&mut resources);
    let (ok, lines) = game
        .run_guest_command_told(sunrise_gameplay_common::game::lan_actions::GuestCommand {
            conn: 2,
            command: "random roll 1..2".into(),
            permission: 2,
            integrated: false,
            quiet: false,
        })
        .expect("the guest is on");
    assert!(ok);
    let mine: Vec<String> = lines.iter().map(Text::plain).collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(
        lines[0].translation_key(),
        Some("commands.random.roll"),
        "{mine:?}"
    );
    assert!(mine[0].starts_with("Roller2 rolled "), "{mine:?}");
    // The other player hears it through the hub, once, with the same line.
    let theirs = chats(&receivers[1]);
    assert_eq!(theirs.len(), 1, "{theirs:?}");
    assert_eq!(theirs[0], mine[0]);
    assert!(theirs[0].ends_with(" (from 1 to 2)"), "{theirs:?}");
    // The rolling guest's own connection was skipped by the broadcast: the one
    // copy it gets is the command's captured output.
    let own = chats(&receivers[0]);
    assert_eq!(
        own.iter().filter(|line| *line == &mine[0]).count(),
        1,
        "{own:?}"
    );
}

/// RandomSequences.forAllSequences: the server's answer to the chat box's
/// completion of `/random reset ` lists the sequences the world has made.
#[test]
#[ignore = "daily"]
fn the_reset_argument_offers_the_sequences_the_world_has() {
    let mut resources = None;
    let (mut game, receivers) = server_with_guests(&mut resources);
    assert!(game.run_command("random value 1..10 minecraft:test"));
    // ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    // `ask_server` request the window's chat box sends for `/random reset `.
    let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
    w.varint(1);
    w.string("random reset ");
    game.guest_suggestions(2, &w.data);
    let offered = suggestions(&receivers[0]);
    assert!(offered.iter().any(|s| s == "minecraft:test"), "{offered:?}");
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
