//! `Commands::feedback(line, broadcast_to_admins)`: vanilla's
//! `CommandSourceStack.sendSuccess(line, true)`, whose `broadcastToAdmins` tells the other
//! operators (`PlayerList.isOp`) under `sendCommandFeedback` and the server log under
//! `logAdminCommands` (#4632).
use mod_api::{Command, PermissionLevel, RegisterCommandsEvent, ServerHost, Text};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize},
    mpsc,
};
use sunrise_game_data::gamerules::Value as Rule;
use sunrise_protocol::packets::{configuration::ClientInformation, play::Clientbound};
use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest};

/// The source (an operator), the `execute as` target (an operator), an operator listed at
/// level 0 and a player who is not an operator.
const SOURCE: u32 = 20;
const TARGET: u32 = 21;
const LEVEL_0_OP: u32 = 22;
const NON_OP: u32 = 23;
const PLAYERS: [(u32, &str); 4] = [
    (SOURCE, "Alex"),
    (TARGET, "Bea"),
    (LEVEL_0_OP, "Cid"),
    (NON_OP, "Dan"),
];

struct Announce;

/// A key in the dedicated console's `BROADCAST` list.
const RELOADED: &str = "commands.whitelist.reloaded";

#[mod_api::__subscriptions]
impl mod_api::Hooks for Announce {
    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event.commands.extend([
            // FTB Teams' `source.sendSuccess(() -> msg, true)`.
            Command::new("announce").executes(|host, _, _| {
                host.commands()
                    .feedback(Text::literal("Disbanded team"), true)
                    .unwrap();
                Ok(1)
            }),
            // A query's `sendSuccess(msg, false)`.
            Command::new("announce_quietly").executes(|host, _, _| {
                host.commands()
                    .feedback(Text::literal("Disbanded team"), false)
                    .unwrap();
                Ok(1)
            }),
            // Translated replies whose key the dedicated console's own commands broadcast
            // (`BROADCAST`): the flag alone decides.
            Command::new("reload_lists").executes(|host, _, _| {
                host.commands()
                    .feedback(Text::translate(RELOADED, vec![]), true)
                    .unwrap();
                Ok(1)
            }),
            Command::new("reload_lists_quietly").executes(|host, _, _| {
                host.commands()
                    .feedback(Text::translate(RELOADED, vec![]), false)
                    .unwrap();
                Ok(1)
            }),
        ]);
    }
}

/// A dedicated server with the `announce` commands and the four players of `PLAYERS`, whose
/// chat each test reads (by conn).
fn with_operators(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &[(u32, mpsc::Receiver<Out>)]),
) {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("announce", Box::new(Announce))].into(),
        || {
            super::native_guest_turn_tests::with_guests(|game| {
                let mut receivers = Vec::new();
                // A dedicated server: its players' commands run at their ops.json level.
                game.set_allow_commands(true);
                let host = game.server.lan.as_mut().unwrap();
                host.settings.guest_commands = true;
                let hub = &mut host.hub;
                hub.config.integrated = false;
                for (conn, name) in PLAYERS {
                    let (tx, rx) = mpsc::channel();
                    let mut guest = Guest::new(
                        Joined {
                            conn,
                            uuid: u128::from(conn),
                            name: name.into(),
                            properties: Vec::new(),
                            info: ClientInformation::new(8),
                            brand: String::new(),
                            sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                            latency: Arc::new(AtomicI32::new(0)),
                            address: "127.0.0.1:1".parse().unwrap(),
                            expected: false,
                        },
                        conn as i32,
                        [3.0, 64.0, 5.0],
                        10,
                    );
                    guest.placed = true;
                    // ops.json: two level-4 operators and one listed at level 0.
                    let level = match conn {
                        SOURCE | TARGET => Some(4),
                        LEVEL_0_OP => Some(0),
                        _ => None,
                    };
                    guest.permission = level.unwrap_or(0);
                    if let Some(level) = level {
                        hub.ops.insert(u128::from(conn), level);
                    }
                    hub.guests.push(guest);
                    receivers.push((conn, rx));
                }
                game.server.next_entity = game.server.next_entity.max(NON_OP);
                game.ensure_datapacks();
                game.build_registries();
                // A dedicated server's command tree (`save-off`), as `run_dedicated` selects it
                // (process-wide: each test runs in its own process).
                sunrise_command::vanilla::set_dedicated_commands(true);
                game.register_native_commands();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let back = game.enter_command_level(sunrise_gameplay_common::game::Dim::Nether);
                assert_eq!(
                    game.dim,
                    sunrise_gameplay_common::game::Dim::Nether,
                    "the fixture has a Nether"
                );
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                if let Some(back) = back {
                    game.enter_command_level(back);
                }
                // A dedicated server's log (`dedicated::log`), which the tests read.
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
                assert!(
                    game.native_run_command("announce"),
                    "the mod's command is registered"
                );
                test(game, &receivers);
            });
        },
    );
}

/// The chat components each player was sent since the last read, by conn.
fn chats(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    receivers: &[(u32, mpsc::Receiver<Out>)],
) -> Vec<(u32, Vec<Value>)> {
    for g in &game.server.lan.as_ref().unwrap().hub.guests {
        g.sender.flush();
    }
    receivers
        .iter()
        .map(|(conn, rx)| {
            let mut lines = Vec::new();
            for sent in rx.try_iter() {
                let bodies = match sent {
                    Out::Packet(p) => vec![p],
                    Out::Packets(p) => p,
                    Out::Disconnect(_) => panic!("unexpected disconnect"),
                };
                for body in bodies {
                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                    let id = reader.varint().unwrap();
                    if let Clientbound::SystemChat(p) =
                        Clientbound::decode(id, reader.rest()).unwrap()
                    {
                        lines.push(sunrise_game_data::scoreboard::tag_to_json(&p.content));
                    }
                }
            }
            (*conn, lines)
        })
        .collect()
}

/// The `[name: Disbanded team]` line every operator gets: `chat.type.admin`, gray and italic,
/// with the display name of the `execute as` entity, reduced to the fields compared.
fn admin(name: &str) -> Value {
    json!({
        "translate": "chat.type.admin",
        "name": name,
        "message": {"text": "Disbanded team"},
        "color": "gray",
        "italic": true,
    })
}

/// A sent line as compared: an admin line's display name as its text alone (a player's name
/// also carries its hover and click events), any other line as sent.
fn shape(line: &Value) -> Value {
    if line["translate"] != "chat.type.admin" {
        return line.clone();
    }
    let with = line["with"]
        .as_array()
        .expect("chat.type.admin has two arguments");
    assert_eq!(with.len(), 2, "{line}");
    json!({
        "translate": line["translate"],
        "name": with[0].as_str().map_or_else(|| with[0]["text"].clone(), Value::from),
        "message": with[1],
        "color": line["color"],
        "italic": line["italic"],
    })
}

fn set_rules(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    send_command_feedback: bool,
    log_admin_commands: bool,
) {
    assert!(
        game.ui
            .rules
            .set("send_command_feedback", Rule::Bool(send_command_feedback))
    );
    assert!(
        game.ui
            .rules
            .set("log_admin_commands", Rule::Bool(log_admin_commands))
    );
}

/// The `broadcastToAdmins` lines the server logged while `f` ran.
fn admin_log(f: impl FnOnce()) -> Vec<String> {
    sunrise_gameplay::game::dedicated::capture_log(f)
        .into_iter()
        .filter(|line| line.starts_with("System chat: ["))
        .collect()
}

/// `execute as` another operator: the source hears the line under `sendCommandFeedback`;
/// every operator but the source (the `execute as` player and the level-0 operator, never the
/// non-operator) gets `[Bea: ...]` under `sendCommandFeedback`; the log gets it under
/// `logAdminCommands`; neither rule gates the other. `broadcast_to_admins` false and a silent
/// source tell no one.
#[test]
#[ignore = "daily"]
fn feedback_tells_the_other_operators_and_the_log_under_their_rules() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_success_broadcast_tests::feedback_tells_the_other_operators_and_the_log_under_their_rules",
        || {
            with_operators(|game, receivers| {
                chats(game, receivers);
                for (feedback, log) in [(true, true), (true, false), (false, true), (false, false)]
                {
                    set_rules(game, feedback, log);
                    let logged = admin_log(|| {
                        game.guest_command(SOURCE, "execute as Bea run announce", None);
                        game.run_guest_commands();
                    });
                    let told = |admin_line: bool| {
                        if admin_line && feedback {
                            vec![admin("Bea")]
                        } else {
                            Vec::new()
                        }
                    };
                    let own = if feedback {
                        vec![json!({"text": "Disbanded team"})]
                    } else {
                        Vec::new()
                    };
                    let sent: Vec<(u32, Vec<Value>)> = chats(game, receivers)
                        .into_iter()
                        .map(|(conn, lines)| (conn, lines.iter().map(shape).collect()))
                        .collect();
                    assert_eq!(
                        sent,
                        vec![
                            (SOURCE, own),
                            (TARGET, told(true)),
                            (LEVEL_0_OP, told(true)),
                            (NON_OP, told(false)),
                        ],
                        "sendCommandFeedback {feedback}, logAdminCommands {log}"
                    );
                    let expected_log = if log {
                        vec!["System chat: [Bea: Disbanded team]".to_owned()]
                    } else {
                        Vec::new()
                    };
                    assert_eq!(
                        logged, expected_log,
                        "sendCommandFeedback {feedback}, logAdminCommands {log}"
                    );
                }

                // The player's own command: `[Alex: ...]`, never back to Alex.
                set_rules(game, true, true);
                let logged = admin_log(|| {
                    game.guest_command(SOURCE, "announce", None);
                    game.run_guest_commands();
                });
                assert_eq!(
                    logged,
                    vec!["System chat: [Alex: Disbanded team]".to_owned()]
                );
                let sent: Vec<(u32, Vec<Value>)> = chats(game, receivers)
                    .into_iter()
                    .map(|(conn, lines)| (conn, lines.iter().map(shape).collect()))
                    .collect();
                assert_eq!(
                    sent,
                    vec![
                        (SOURCE, vec![json!({"text": "Disbanded team"})]),
                        (TARGET, vec![admin("Alex")]),
                        (LEVEL_0_OP, vec![admin("Alex")]),
                        (NON_OP, Vec::new()),
                    ]
                );

                // `broadcast_to_admins` false: the source alone, nothing logged.
                let logged = admin_log(|| {
                    game.guest_command(SOURCE, "announce_quietly", None);
                    game.run_guest_commands();
                });
                assert_eq!(logged, Vec::<String>::new());
                assert_eq!(
                    chats(game, receivers),
                    vec![
                        (SOURCE, vec![json!({"text": "Disbanded team"})]),
                        (TARGET, Vec::new()),
                        (LEVEL_0_OP, Vec::new()),
                        (NON_OP, Vec::new()),
                    ]
                );

                // A silent source (`withSuppressedOutput`) sends neither part.
                let alex = game.player(u128::from(SOURCE)).unwrap().entity();
                let logged = admin_log(|| {
                    assert!(alex.run_command(game, "announce", PermissionLevel::Owners, true));
                });
                assert_eq!(logged, Vec::<String>::new());
                assert!(
                    chats(game, receivers)
                        .iter()
                        .all(|(_, lines)| lines.is_empty()),
                    "a silent source told someone"
                );
            });
        },
    );
}

/// A command block that tracks output informs the operators under `commandBlockOutput`: the
/// log records `[@: ...]` while `sendCommandFeedback` is off (and the block keeps no output);
/// with `commandBlockOutput` off only the block's own output has the line.
#[test]
#[ignore = "daily"]
fn a_tracking_command_block_is_logged_while_feedback_is_off() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_success_broadcast_tests::a_tracking_command_block_is_logged_while_feedback_is_off",
        || {
            use sunrise_core::position::BlockPos;
            use sunrise_level::{chunk::block_entity::BlockEntity, command_block::CommandBlock};
            with_operators(|game, receivers| {
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:command_block")
                    .unwrap();
                game.world.set(1, 64, 1, state).unwrap();
                let last_output = |game: &sunrise_gameplay::game::ServerGame<'_>| match game
                    .world
                    .block_entities
                    .get(&(1, 64, 1))
                {
                    Some(BlockEntity::CommandBlock(block)) => {
                        block.last_output.as_ref().map(|line| line.plain())
                    }
                    _ => panic!("the command block is gone"),
                };
                let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str| {
                    game.world.block_entities.insert(
                        (1, 64, 1),
                        BlockEntity::CommandBlock(Box::new(CommandBlock {
                            command: command.into(),
                            condition_met: true,
                            track_output: true,
                            ..Default::default()
                        })),
                    );
                    admin_log(|| game.command_block_tick(BlockPos::new(1, 64, 1)))
                };
                chats(game, receivers);

                set_rules(game, false, true);
                assert!(game.ui.rules.set("command_block_output", Rule::Bool(true)));
                assert_eq!(
                    run(game, "announce"),
                    vec!["System chat: [@: Disbanded team]".to_owned()]
                );
                // `execute in` another level: the block, read in its own level, still informs
                // the operators (`BaseCommandBlock.shouldInformAdmins`).
                assert_eq!(
                    run(game, "execute in minecraft:the_nether run announce"),
                    vec!["System chat: [@: Disbanded team]".to_owned()]
                );
                assert_eq!(last_output(game), None);
                game.send_admin_feedback();
                assert!(
                    chats(game, receivers)
                        .iter()
                        .all(|(_, lines)| lines.is_empty()),
                    "an operator heard the block with sendCommandFeedback off"
                );

                // A block's line still queued when an operator's command drains the queue:
                // it keeps the block as its source, so that operator hears it too.
                set_rules(game, true, true);
                assert!(game.ui.rules.set("command_block_output", Rule::Bool(true)));
                chats(game, receivers);
                run(game, "announce");
                game.guest_command(SOURCE, "announce_quietly", None);
                game.run_guest_commands();
                let sent: Vec<(u32, Vec<Value>)> = chats(game, receivers)
                    .into_iter()
                    .map(|(conn, lines)| (conn, lines.iter().map(shape).collect()))
                    .collect();
                assert_eq!(
                    sent,
                    vec![
                        (SOURCE, vec![json!({"text": "Disbanded team"}), admin("@")]),
                        (TARGET, vec![admin("@")]),
                        (LEVEL_0_OP, vec![admin("@")]),
                        (NON_OP, Vec::new()),
                    ]
                );

                // The block does not inform the operators: its own output only.
                set_rules(game, true, true);
                assert!(game.ui.rules.set("command_block_output", Rule::Bool(false)));
                assert_eq!(run(game, "announce"), Vec::<String>::new());
                assert!(
                    last_output(game).is_some_and(|line| line.ends_with("Disbanded team")),
                    "{:?}",
                    last_output(game)
                );
                game.send_admin_feedback();
                assert!(
                    chats(game, receivers)
                        .iter()
                        .all(|(_, lines)| lines.is_empty())
                );
            });
        },
    );
}

/// The queued ops lines, sent (`send_admin_feedback`) and shaped, by conn.
fn admin_lines(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    receivers: &[(u32, mpsc::Receiver<Out>)],
) -> Vec<(u32, Vec<Value>)> {
    game.send_admin_feedback();
    chats(game, receivers)
        .into_iter()
        .map(|(conn, lines)| (conn, lines.iter().map(shape).collect()))
        .collect()
}

/// `execute in` another level `as` a mob there: the operators' line carries that mob's name
/// (`getDisplayName` is the entity's, wherever it is), read in the level the command executes
/// in, not the source's own.
#[test]
#[ignore = "daily"]
fn an_entity_in_another_level_names_the_admin_line() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_success_broadcast_tests::an_entity_in_another_level_names_the_admin_line",
        || {
            with_operators(|game, receivers| {
                assert!(game.native_run_command(
                    "execute in minecraft:the_nether run summon minecraft:pig 0.5 64 0.5 {CustomName:\"Porky\",NoAI:1b,NoGravity:1b}"
                ));
                chats(game, receivers);
                set_rules(game, true, true);
                let logged = admin_log(|| {
                    game.guest_command(
                        SOURCE,
                        "execute in minecraft:the_nether as @e[type=minecraft:pig,limit=1] run announce",
                        None,
                    );
                    game.run_guest_commands();
                });
                assert_eq!(
                    logged,
                    vec!["System chat: [Porky: Disbanded team]".to_owned()]
                );
                assert_eq!(
                    admin_lines(game, receivers),
                    vec![
                        (SOURCE, vec![json!({"text": "Disbanded team"})]),
                        (TARGET, vec![admin("Porky")]),
                        (LEVEL_0_OP, vec![admin("Porky")]),
                        (NON_OP, Vec::new()),
                    ]
                );
            });
        },
    );
}

/// A console or RCON command's output is logged once and broadcast once: a mod line with
/// `broadcast_to_admins` false tells no operator, a broadcast one tells each operator once
/// (`[Server: ...]`, never logged as the console's own admin line), and a vanilla command whose
/// reply the console broadcasts (`save-off` through `execute`) is not broadcast a second time.
#[test]
#[ignore = "daily"]
fn console_output_is_logged_and_broadcast_once() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_success_broadcast_tests::console_output_is_logged_and_broadcast_once",
        || {
            use sunrise_server_files::user_lists::{BanList, IpBanList, OpList, WhiteList};
            with_operators(|game, receivers| {
                chats(game, receivers);
                set_rules(game, true, true);
                let console =
                    |game: &mut sunrise_gameplay::game::ServerGame<'_>, rcon: bool, line: &str| {
                        game.server.console_broadcast = Some(true);
                        game.server.rcon_source = rcon;
                        let logged = sunrise_gameplay::game::dedicated::capture_log(|| {
                            game.console_dispatch(line)
                        });
                        game.server.console_broadcast = None;
                        game.server.rcon_source = false;
                        logged
                    };
                let nobody = vec![
                    (SOURCE, Vec::new()),
                    (TARGET, Vec::new()),
                    (LEVEL_0_OP, Vec::new()),
                    (NON_OP, Vec::new()),
                ];
                let every_op = |line: Value| {
                    vec![
                        (SOURCE, vec![line.clone()]),
                        (TARGET, vec![line.clone()]),
                        (LEVEL_0_OP, vec![line]),
                        (NON_OP, Vec::new()),
                    ]
                };

                // `broadcast_to_admins` false: the console's own line, no operator, from the
                // console and from RCON (whose reply is that one line).
                for rcon in [false, true] {
                    assert_eq!(
                        console(game, rcon, "announce_quietly"),
                        vec!["System chat: Disbanded team".to_owned()],
                        "rcon {rcon}"
                    );
                    assert_eq!(admin_lines(game, receivers), nobody, "rcon {rcon}");
                }

                // Broadcast: each operator once.
                assert_eq!(
                    console(game, false, "announce"),
                    vec!["System chat: Disbanded team".to_owned()]
                );
                assert_eq!(admin_lines(game, receivers), every_op(admin("Server")));

                // A vanilla reply the console broadcasts, through the dispatcher: once. `save-off`
                // reads the dedicated server's lists.
                let home = tempfile::tempdir().unwrap();
                game.server.server_lists.0 = Some(Arc::new(std::sync::Mutex::new(
                    sunrise_gameplay::game::dedicated::Lists {
                        ops: OpList::new(home.path().join("ops.json")),
                        white: WhiteList::new(home.path().join("whitelist.json")),
                        bans: BanList::new(home.path().join("banned-players.json")),
                        ip_bans: IpBanList::new(home.path().join("banned-ips.json")),
                        use_whitelist: false,
                        enforce_whitelist: false,
                        op_level: 4,
                        player_idle_timeout: 0,
                        stop: false,
                        autosave: true,
                    },
                )));
                let logged = console(game, false, "execute run save-off");
                assert_eq!(logged.len(), 1, "{logged:?}");
                let sent = admin_lines(game, receivers);
                for (conn, lines) in &sent {
                    let expected = usize::from(*conn != NON_OP);
                    assert_eq!(lines.len(), expected, "{conn}: {sent:?}");
                    if let Some(line) = lines.first() {
                        assert_eq!(
                            line["message"]["translate"], "commands.save.disabled",
                            "{line}"
                        );
                        assert_eq!(line["name"], "Server", "{line}");
                    }
                }
                assert_eq!(console(game, false, "execute run save-on").len(), 1);
                let sent = admin_lines(game, receivers);
                let told: usize = sent.iter().map(|(_, lines)| lines.len()).sum();
                assert_eq!(
                    told, 3,
                    "one line for each of the three operators: {sent:?}"
                );

                // A mod's translated line with a key the console's own commands broadcast:
                // false tells no operator, true tells each once, from the console and RCON.
                for rcon in [false, true] {
                    assert_eq!(
                        console(game, rcon, "reload_lists_quietly").len(),
                        1,
                        "rcon {rcon}"
                    );
                    assert_eq!(admin_lines(game, receivers), nobody, "rcon {rcon}");
                    assert_eq!(console(game, rcon, "reload_lists").len(), 1, "rcon {rcon}");
                    let sent = admin_lines(game, receivers);
                    let name = if rcon { "Rcon" } else { "Server" };
                    for (conn, lines) in &sent {
                        assert_eq!(
                            lines.len(),
                            usize::from(*conn != NON_OP),
                            "{conn}: {sent:?}"
                        );
                        if let Some(line) = lines.first() {
                            assert_eq!(line["message"]["translate"], RELOADED, "{line}");
                            assert_eq!(line["name"], name, "{line}");
                        }
                    }
                }
            });
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
