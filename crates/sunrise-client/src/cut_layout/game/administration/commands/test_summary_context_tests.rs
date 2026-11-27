//! Added after the original old-fail module: context boundaries, not new runtime breadth.
use super::*;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_game_data::text::Text;
use sunrise_gameplay::game::commands::execute::Source;
use sunrise_gameplay::game::game_tests::Out;
use sunrise_gameplay::game::game_tests::{Grid, Info, RunnerOptions, SummaryKind, summary_line};
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use sunrise_sim::lan::data::component_nbt;
use sunrise_sim::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use sunrise_world::worldgen::structure::Rotation;

fn chats(receiver: &mpsc::Receiver<sunrise_sim::lan::Out>) -> Vec<sunrise_core::nbt::Tag> {
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
                result.push(p.content);
            }
        }
    }
    result
}

#[test]
#[ignore = "daily"]
fn added_issuer_actor_missing_guest_and_restoration_controls() {
    let mut resources = None;
    let mut game = tests::server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.level.health = 20.0;
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::BlockPos::new(0, 0, 0).chunk())
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
    assert!(game.run_command("summon pig 2.5 200.0 2.5 {UUID:[I;0,0,0,99],NoAI:1b}"));
    let actor = game
        .mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some([0, 0, 0, 99]))
        .unwrap()
        .id;
    for line in [
        "scoreboard objectives add context dummy",
        "scoreboard players set result context 99",
        "scoreboard players set success context 99",
    ] {
        assert!(game.run_command(line));
    }
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 200.0, 1.5)).unwrap());
    game.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "test-summary-context".into(), 4)
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
                name: format!("Issuer{conn}"),
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
    // Added actor/issuer separation: actual execute-as, with the issuer unchanged.
    assert!(game.run_guest_command(sunrise_gameplay_common::game::lan_actions::GuestCommand {
        conn: 2, command: "execute as 00000000-0000-0000-0000-000000000063 store result score result context store success score success context run test run minecraft:always_pass".into(),
        permission: 2, integrated: false, quiet: false }));
    let context = game
        .server
        .game_tests
        .0
        .runner
        .as_ref()
        .unwrap()
        .summary_source
        .clone()
        .unwrap();
    assert_eq!(context.source.entity, Some(Target::Mob(actor)));
    assert_eq!(context.recipient, Some(Target::Guest(2)));
    assert_eq!(
        game.score.board.score("result", "context").unwrap().value,
        1
    );
    assert_eq!(
        game.score.board.score("success", "context").unwrap().value,
        1
    );
    for _ in 0..8 {
        game.tick_game_tests();
    }
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    let _ = chats(&receivers[0]);
    let _ = chats(&receivers[1]);
    assert!(
        game.run_guest_command(sunrise_gameplay_common::game::lan_actions::GuestCommand {
            conn: 2,
            command: "test run minecraft:always_pass".into(),
            permission: 2,
            integrated: false,
            quiet: false
        })
    );
    assert_eq!(
        game.server
            .game_tests
            .0
            .runner
            .as_ref()
            .unwrap()
            .summary_source
            .as_ref()
            .unwrap()
            .source
            .entity,
        Some(Target::Guest(2)),
        "added transient Player-alias normalization control"
    );
    game.flush_game_test_output();
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    let _ = chats(&receivers[0]);
    let _ = chats(&receivers[1]);
    game.enter_command_level(Dim::Nether);
    let outer = Source {
        dimension: Dim::Nether,
        pos: [31.0, 200.0, 31.0],
        silent: true,
        ..game.player_source()
    };
    game.server.command_source = Some(outer.clone());
    game.server.chat_capture = Some(Vec::new());
    let before = (game.server.exec.0.commands, game.server.exec.0.callback);
    let marker = Text::literal("issuer-only-context-control");
    let component = component_nbt(&marker.component());
    game.test_source_output(
        &context,
        SummaryKind::Success { broadcast: false },
        marker.clone(),
    );
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    assert_eq!(chats(&receivers[0]), [component]);
    assert!(chats(&receivers[1]).is_empty());
    assert!(game.server.chat_capture.as_ref().unwrap().is_empty());
    assert_eq!(game.dim, Dim::Nether);
    assert_eq!(game.server.lan_acting, None);
    assert_eq!(
        game.player.as_ref().unwrap().position,
        Vec3::new(1.5, 200.0, 1.5)
    );
    assert_eq!(
        format!("{:?}", game.server.command_source.as_ref().unwrap()),
        format!("{outer:?}")
    );
    assert_eq!(
        before,
        (game.server.exec.0.commands, game.server.exec.0.callback)
    );
    // Added missing-recipient control: no host fallback or operator broadcast.
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .retain(|g| g.conn != 2);
    game.test_source_output(&context, SummaryKind::Success { broadcast: true }, marker);
    game.server
        .game_tests
        .0
        .outbox
        .push(Out::Coordinates(context, vec![Dim::Overworld]));
    game.flush_game_test_output();
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    assert!(chats(&receivers[1]).is_empty());
    assert!(game.server.chat_capture.as_ref().unwrap().is_empty());
    assert_eq!(game.dim, Dim::Nether);
    assert_eq!(game.server.lan_acting, None);
    assert_eq!(
        format!("{:?}", game.server.command_source.as_ref().unwrap()),
        format!("{outer:?}")
    );
    assert_eq!(
        before,
        (game.server.exec.0.commands, game.server.exec.0.callback)
    );
    assert_eq!(
        game.score.board.score("result", "context").unwrap().value,
        1
    );
    assert_eq!(
        game.score.board.score("success", "context").unwrap().value,
        1
    );
    // Added GUI/verify path-selection controls: no retained command context.
    assert!(
        RunnerOptions::verify(Grid::new((8, 8), 10))
            .summary_source
            .is_none()
    );
    let info = Info::new(
        999,
        "minecraft:always_pass".into(),
        game.test_def("minecraft:always_pass").unwrap(),
        Some((8, 200, 11)),
        Dim::Overworld,
        Rotation::None,
    );
    game.track_and_start_runner(vec![info], false, None, false);
    assert!(
        game.server
            .game_tests
            .0
            .runner
            .as_ref()
            .unwrap()
            .summary_source
            .is_none()
    );
    assert!(matches!(
        summary_line(
            None,
            SummaryKind::Success { broadcast: true },
            Text::literal("GUI path")
        ),
        Out::Console { .. }
    ));
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
