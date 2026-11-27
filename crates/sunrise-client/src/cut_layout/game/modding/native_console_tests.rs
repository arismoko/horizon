//! The console for native mods: logged lines reach `console_line`, and a command's feedback
//! comes back from `run_command_with_output`.
use super::{native_entity_refs::tests::with_game, native_guest_turn_tests::with_guests};
use mod_api::{
    Command, CommandFeedback, CommandResult, Commands, ConsoleLineEvent, Hooks,
    RegisterCommandsEvent, ServerHost, Text, text::TextContent,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_level::logging::TappedLine;

type HeardLine = (String, String, String, u64);

struct Mirror {
    overwrite: bool,
    heard: Arc<Mutex<Vec<HeardLine>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Mirror {
    fn console_line(&self, _: &mut dyn ServerHost, event: &mut ConsoleLineEvent<'_>) {
        if self.overwrite {
            event.dropped = 99;
        }
        self.heard.lock().unwrap().push((
            event.level.to_owned(),
            event.thread.to_owned(),
            event.message.to_owned(),
            event.dropped,
        ));
    }
}

#[test]
#[ignore = "daily"]
fn each_console_listener_gets_the_original_overflow_count() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        mod_api::ModEntry::new(
            "first",
            Box::new(Mirror {
                overwrite: true,
                heard: heard.clone(),
            }),
        ),
        mod_api::ModEntry::new(
            "second",
            Box::new(Mirror {
                overwrite: false,
                heard: heard.clone(),
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // The queue's own tests cover retention and overflow. Drive the hook with local
            // batches so unrelated threads cannot add lines or consume the first report.
            let line = |level, message: &str, dropped| TappedLine {
                level,
                thread: "Server thread".into(),
                message: message.into(),
                dropped,
            };
            game.dispatch_console_lines(vec![
                line(log::Level::Warn, "first in batch", 3),
                line(log::Level::Info, "later in batch", 0),
            ]);
            game.dispatch_console_lines(vec![line(log::Level::Info, "next batch", 0)]);
            let heard = heard.lock().unwrap();
            let heard: Vec<_> = heard
                .iter()
                .map(|(level, thread, message, dropped)| {
                    (level.as_str(), thread.as_str(), message.as_str(), *dropped)
                })
                .collect();
            assert_eq!(
                heard,
                [
                    ("WARN", "Server thread", "first in batch", 99),
                    ("WARN", "Server thread", "first in batch", 3),
                    ("INFO", "Server thread", "later in batch", 99),
                    ("INFO", "Server thread", "later in batch", 0),
                    ("INFO", "Server thread", "next batch", 99),
                    ("INFO", "Server thread", "next batch", 0),
                ]
            );
        });
    });
}

/// The console's feedback comes back as plain text with the result of `run_command`. A command
/// that ran and failed is `ran: false` with its message, and one leading slash is dropped.
#[test]
#[ignore = "daily"]
fn a_captured_command_returns_its_feedback() {
    with_guests(|game| {
        let result = game.native_run_command_with_output("/time query gametime");
        assert!(result.success);
        assert_eq!(result.feedback.len(), 1, "{result:?}");
        assert!(
            result.feedback[0].starts_with("The game time is"),
            "{result:?}"
        );
        let result = game.native_run_command_with_output(" time query gametime ");
        assert!(result.success && result.feedback.len() == 1, "{result:?}");
        let result = game.native_run_command_with_output("gamerule nosuchrule");
        assert!(!result.success);
        assert!(!result.feedback.is_empty());
        assert!(
            game.server.chat_capture.is_none(),
            "the outer capture is put back"
        );
        // The same line through `run_command`'s path answers as the captured one does.
        assert!(game.native_run_command("time query gametime"));
        assert!(!game.native_run_command("gamerule nosuchrule"));
    });
}

/// The dedicated console's own commands are not reached: they answer as a command that failed.
#[test]
#[ignore = "daily"]
fn the_dedicated_consoles_own_commands_are_not_reached() {
    with_guests(|game| {
        for line in ["stop", "op nobody", "whitelist list", "ban nobody"] {
            let result = game.native_run_command_with_output(line);
            assert!(!result.success, "{line}: {result:?}");
            assert!(!result.feedback.is_empty(), "{line}");
        }
    });
}

/// A blank line (empty, spaces, a lone slash, a slash and spaces) and no server open to players
/// run nothing and answer as `run_command` does: `ran` false and no lines.
#[test]
#[ignore = "daily"]
fn a_blank_line_or_no_server_runs_nothing() {
    with_guests(|game| {
        for blank in ["", "   ", "/", " / ", "/   "] {
            assert_eq!(
                game.native_run_command_with_output(blank),
                CommandResult::default(),
                "{blank:?}"
            );
        }
        assert!(game.server.chat_capture.is_none(), "nothing was captured");
        let host = game.server.lan.take();
        assert_eq!(
            game.native_run_command_with_output("time query gametime"),
            CommandResult::default()
        );
        assert!(!game.native_run_command("time query gametime"));
        game.server.lan = host;
        assert!(
            game.native_run_command_with_output("time query gametime")
                .success
        );
    });
}

struct FeedbackCommand;

#[mod_api::__subscriptions]
impl Hooks for FeedbackCommand {
    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        event
            .commands
            .push(Command::new("feedback_probe").executes(|host, _, _| {
                let mut text = Text::literal("€".repeat(1500));
                text.style.bold = Some(true);
                host.commands().feedback(text, false).unwrap();
                for index in 1..300 {
                    host.commands()
                        .feedback(Text::literal(format!("line {index}")), false)
                        .unwrap();
                    if index == 150 {
                        let inner = host
                            .commands()
                            .run_command_with_output("time query gametime");
                        assert!(inner.success);
                        assert_eq!(inner.feedback.len(), 1);
                    }
                }
                Err(Text::literal("probe failed").into())
            }));
    }
}

#[test]
#[ignore = "daily"]
fn native_console_delivers_all_source_feedback_in_order() {
    use sunrise_game_data::gamerules::{Value, vanilla::SEND_COMMAND_FEEDBACK};
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "feedback_probe",
            Box::new(FeedbackCommand),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.build_registries();
                game.register_native_commands();
                assert!(
                    game.ui
                        .rules
                        .set("send_command_feedback", Value::Bool(false))
                );
                assert!(!game.ui.rules.bool_at(SEND_COMMAND_FEEDBACK));

                let (send, receive) = std::sync::mpsc::channel();
                assert!(!Commands::run_command_with_feedback(
                    game,
                    "/feedback_probe",
                    send
                ));
                let mut output = receive.try_iter();
                let long = "€".repeat(1500);
                let Some(Ok(CommandFeedback::Success(first))) = output.next() else {
                    panic!("expected the command's first success message");
                };
                assert_eq!(first.content, TextContent::Literal(long.clone()));
                assert_eq!(first.style.bold, Some(true));
                for index in 1..300 {
                    let Some(Ok(CommandFeedback::Success(line))) = output.next() else {
                        panic!("missing success message {index}");
                    };
                    assert_eq!(line.content, TextContent::Literal(format!("line {index}")));
                }
                let Some(Ok(CommandFeedback::Failure(error))) = output.next() else {
                    panic!("a failed command must deliver its error after earlier successes");
                };
                assert!(
                    matches!(error.style.color.as_deref(), Some("red" | "#FF5555")),
                    "the error keeps its red style"
                );
                assert_eq!(
                    output.next(),
                    None,
                    "the nested command's output stays separate"
                );

                let plain = Commands::run_command_with_output(game, "feedback_probe");
                assert!(!plain.success);
                assert_eq!(plain.feedback.len(), 301);
                assert_eq!(plain.feedback[0], long);
                assert_eq!(plain.feedback[299], "line 299");
                assert!(!plain.feedback[300].is_empty());
            })
        },
    );
}

fn translation_key(text: &Text) -> Option<&str> {
    if let TextContent::Translatable { key, .. } = &text.content {
        return Some(key.as_str());
    }
    text.children.iter().find_map(translation_key)
}

fn guest_chats(
    receiver: &std::sync::mpsc::Receiver<sunrise_client::lan::Out>,
) -> Vec<sunrise_core::nbt::Tag> {
    use sunrise_protocol::packets::play::Clientbound;
    let mut result = Vec::new();
    for sent in receiver.try_iter() {
        let bodies = match sent {
            sunrise_client::lan::Out::Packet(p) => vec![p],
            sunrise_client::lan::Out::Packets(p) => p,
            sunrise_client::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
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

/// The disguised-chat messages a guest was sent, as plain text.
fn guest_disguised_chats(
    receiver: &std::sync::mpsc::Receiver<sunrise_client::lan::Out>,
) -> Vec<String> {
    use sunrise_client::scoreboard::{comp_text, tag_to_json};
    use sunrise_protocol::packets::play::Clientbound;
    let mut result = Vec::new();
    for sent in receiver.try_iter() {
        let bodies = match sent {
            sunrise_client::lan::Out::Packet(p) => vec![p],
            sunrise_client::lan::Out::Packets(p) => p,
            sunrise_client::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Clientbound::DisguisedChat(p) = Clientbound::decode(id, reader.rest()).unwrap() {
                result.push(comp_text(&tag_to_json(&p.message)).plain());
            }
        }
    }
    result
}

struct FailGameTest;

impl mod_api::GameTestFunction for FailGameTest {
    fn run(&self, _: &mut dyn ServerHost, _: &mod_api::GameTestContext) -> mod_api::GameTestResult {
        mod_api::GameTestResult::Fail("probe failed".into())
    }
}

struct DelayedSummaryTests;

#[mod_api::__subscriptions]
impl Hooks for DelayedSummaryTests {
    fn register_game_tests(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::RegisterGameTestsEvent<'_>,
    ) {
        event
            .functions
            .register("feedback_probe:fail", Arc::new(FailGameTest))
            .unwrap();
        event
            .functions
            .register_test(
                "feedback_probe:required_fail",
                mod_api::GameTestDefinition::new(
                    "minecraft:empty".into(),
                    "feedback_probe:fail".into(),
                ),
            )
            .unwrap();
        let mut optional = mod_api::GameTestDefinition::new(
            "minecraft:empty".into(),
            "feedback_probe:fail".into(),
        );
        optional.required = false;
        event
            .functions
            .register_test("feedback_probe:optional_fail", optional)
            .unwrap();
    }
}

/// `TestCommand.trackAndStartRunner` / `TestSummaryDisplayer`: delayed summary lines
/// reach the issuing native console source after the command returns, then the channel closes.
/// `run_command_with_output` returns only the synchronous snapshot collected during dispatch.
#[test]
#[ignore = "daily"]
fn native_console_delivers_delayed_game_test_summary_and_closes() {
    use std::sync::mpsc::TryRecvError;
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "feedback_probe",
            Box::new(DelayedSummaryTests),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.build_registries();
                game.reload_datapacks(false, &[]).unwrap();
                let layout = game.world.layout();
                for cx in -2..=2 {
                    for cz in -2..=2 {
                        game.world.chunks.insert(
                            (cx, cz),
                            Arc::new(sunrise_client::chunk::Chunk::with_layout(cx, cz, layout)),
                        );
                        let _ = game
                            .block_ticks
                            .add_empty_chunk(sunrise_core::position::ChunkPos::new(cx, cz));
                    }
                }
                let (send, receive) = std::sync::mpsc::channel();
                assert!(Commands::run_command_with_feedback(
                    game,
                    "test run feedback_probe:*",
                    send,
                ));
                let sync_messages: Vec<_> = receive.try_iter().collect();
                assert_eq!(sync_messages.len(), 2, "{sync_messages:?}");
                assert!(matches!(
                    &sync_messages[0],
                    Ok(CommandFeedback::Success(text))
                        if translation_key(text) == Some("commands.test.run.running")
                ));
                assert!(matches!(
                    &sync_messages[1],
                    Ok(CommandFeedback::Success(text))
                        if translation_key(text) == Some("commands.test.batch.starting")
                ));
                assert!(
                    matches!(receive.try_recv(), Err(TryRecvError::Empty)),
                    "the delayed summary listener keeps the channel open until completion"
                );

                for _ in 0..8 {
                    game.tick_game_tests();
                }
                let delayed: Vec<_> = receive.try_iter().collect();
                assert_eq!(delayed.len(), 4, "{delayed:?}");
                assert!(matches!(
                    &delayed[0],
                    Ok(CommandFeedback::Success(text))
                        if translation_key(text) == Some("commands.test.summary")
                ));
                let Ok(CommandFeedback::Failure(failed)) = &delayed[1] else {
                    panic!("expected required-failure summary: {:?}", delayed[1]);
                };
                assert!(
                    matches!(failed.style.color.as_deref(), Some("red" | "#FF5555")),
                    "required failure keeps red style: {failed:?}"
                );
                assert_eq!(
                    translation_key(failed),
                    Some("commands.test.summary.failed")
                );
                assert!(matches!(
                    &delayed[2],
                    Ok(CommandFeedback::System(text))
                        if translation_key(text) == Some("commands.test.summary.optional_failed")
                ));
                let Ok(CommandFeedback::Success(coordinates)) = &delayed[3] else {
                    panic!("expected a successful coordinates link: {:?}", delayed[3]);
                };
                let TextContent::Translatable { key, arguments, .. } = &coordinates.content else {
                    panic!("expected bracketed coordinates: {coordinates:?}");
                };
                assert_eq!(key, "chat.square_brackets");
                let [coordinates_arg] = arguments.as_slice() else {
                    panic!("expected one component inside the brackets: {coordinates:?}");
                };
                assert_eq!(
                    translation_key(coordinates_arg),
                    Some("test.run.coordinates")
                );
                assert!(
                    matches!(receive.try_recv(), Err(TryRecvError::Disconnected)),
                    "the channel closes once the runner finishes and flushes its summary"
                );

                let plain =
                    Commands::run_command_with_output(game, "test run minecraft:always_pass");
                assert!(plain.success);
                assert_eq!(plain.feedback.len(), 2, "{plain:?}");
                for _ in 0..8 {
                    game.tick_game_tests();
                }
            })
        },
    );
}

/// `MsgCommand.sendMessage` -> `CommandSourceStack.sendChatMessage`: a non-player native
/// console source receives the outgoing whisper confirmation as `CommandFeedback::System`,
/// separately from the target player's incoming message.
#[test]
#[ignore = "daily"]
fn native_console_msg_routes_outgoing_confirmation_to_source() {
    use std::sync::mpsc::TryRecvError;
    with_guests(|game| {
        game.build_registries();
        let (tx12, rx12) = std::sync::mpsc::channel();
        let (tx13, rx13) = std::sync::mpsc::channel();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|guest| {
                let tx = if guest.conn == 12 {
                    tx12.clone()
                } else {
                    tx13.clone()
                };
                guest.sender = sunrise_client::lan::PlayerSender::new(tx, Arc::default());
            });

        let (send, receive) = std::sync::mpsc::channel();
        assert!(Commands::run_command_with_feedback(
            game,
            "msg guest12 hello there",
            send,
        ));
        for guest in &game.server.lan.as_ref().unwrap().hub.guests {
            guest.sender.flush();
        }
        let messages: Vec<_> = receive.try_iter().collect();
        assert_eq!(messages.len(), 1, "{messages:?}");
        let Ok(CommandFeedback::System(outgoing)) = &messages[0] else {
            panic!("expected outgoing system confirmation: {:?}", messages[0]);
        };
        assert_eq!(
            translation_key(outgoing),
            Some("commands.message.display.outgoing")
        );
        assert_eq!(outgoing.style.italic, Some(true));
        assert!(matches!(
            receive.try_recv(),
            Err(TryRecvError::Disconnected)
        ));

        // The target's copy is private chat with the incoming whisper type (`Hub::whisper`'s
        // delivery, which respects the recipient's chat visibility), not system chat.
        let received12 = guest_disguised_chats(&rx12);
        assert_eq!(received12, ["hello there"]);
        assert!(
            guest_chats(&rx12).is_empty(),
            "no system chat for the target"
        );
        assert!(guest_chats(&rx13).is_empty());
        assert!(guest_disguised_chats(&rx13).is_empty());

        let plain = Commands::run_command_with_output(game, "msg guest12 second whisper");
        assert!(plain.success);
        assert_eq!(plain.feedback.len(), 1, "{plain:?}");
        assert!(plain.feedback[0].contains("guest12"), "{plain:?}");
        assert!(plain.feedback[0].contains("second whisper"), "{plain:?}");
    });
}

/// `MinecraftServer.createCommandSourceStack`: direct native console commands run in the
/// world spawn level (`findRespawnDimension`) even when another dimension is active, and
/// restore the previous level afterwards.
#[test]
#[ignore = "daily"]
fn native_console_runs_in_world_spawn_level_and_restores_dimension() {
    use std::sync::mpsc::TryRecvError;
    use sunrise_gameplay_common::game::Dim;
    with_guests(|game| {
        game.build_registries();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let overworld_layout = game.world.layout();
        game.world.chunks.insert(
            (0, 0),
            Arc::new(sunrise_client::chunk::Chunk::with_layout(
                0,
                0,
                overworld_layout,
            )),
        );
        let _ = game
            .block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0));

        game.enter_command_level(Dim::Nether).unwrap();
        let nether_layout = game.world.layout();
        game.world.chunks.insert(
            (0, 0),
            Arc::new(sunrise_client::chunk::Chunk::with_layout(
                0,
                0,
                nether_layout,
            )),
        );
        let _ = game
            .block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0));
        assert_eq!(game.dim, Dim::Nether);

        let (send, receive) = std::sync::mpsc::channel();
        assert!(Commands::run_command_with_feedback(
            game,
            "setblock 1 64 1 minecraft:stone",
            send,
        ));
        assert_eq!(game.dim, Dim::Nether, "previous dimension must be restored");
        assert_ne!(
            game.world.block(1, 64, 1),
            stone,
            "nether must stay untouched"
        );
        let messages: Vec<_> = receive.try_iter().collect();
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(matches!(
            &messages[0],
            Ok(CommandFeedback::Success(text))
                if translation_key(text) == Some("commands.setblock.success")
        ));
        assert!(matches!(
            receive.try_recv(),
            Err(TryRecvError::Disconnected)
        ));

        game.enter_command_level(Dim::Overworld).unwrap();
        assert_eq!(
            game.world.block(1, 64, 1),
            stone,
            "command must run in the world spawn level"
        );
    });
}

/// A call made while another capture is open captures its own lines, and the outer capture
/// keeps what it had and goes on.
#[test]
#[ignore = "daily"]
fn a_nested_capture_leaves_the_outer_one_as_it_was() {
    with_guests(|game| {
        let outer = game
            .server
            .chat_capture
            .replace(vec![sunrise_game_data::text::Text::literal("outer line")]);
        let result = game.native_run_command_with_output("time query gametime");
        assert!(result.success && result.feedback.len() == 1, "{result:?}");
        let kept: Vec<String> = game
            .server
            .chat_capture
            .take()
            .unwrap()
            .iter()
            .map(|t| t.plain())
            .collect();
        assert_eq!(kept, ["outer line"]);
        game.server.chat_capture = outer;
    });
}
