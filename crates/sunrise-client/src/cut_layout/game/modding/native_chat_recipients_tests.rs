//! `chat_recipient`: asked once for each placed receiver before a public line is sent, and a
//! receiver it hides is sent nothing while the others still get the line.
use super::native_entity_refs::tests::{run_fresh_cases, with_game};
use mod_api::{ChatRecipientEvent, CommandResult, Flow, Hooks, PermissionLevel, ServerHost};
use std::sync::{Arc, Mutex, atomic::AtomicI32, mpsc};
use sunrise_core::position::{BlockPos, ChunkPos};
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::{configuration::ClientInformation, play};
use sunrise_sim::lan::hub::chat::ChatMessage;
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest};

type Asked = Arc<Mutex<Vec<(u128, String, String, u128)>>>;

/// Hides every line that says "secret" from guest 13, and records what it is asked.
struct Ignorer(Asked);

#[mod_api::__subscriptions]
impl Hooks for Ignorer {
    fn chat_recipient(
        &self,
        _host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push((
            event.sender,
            event.sender_name.to_owned(),
            event.message.to_owned(),
            event.recipient,
        ));
        if event.message.contains("secret") && event.recipient == 13 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

struct Echo {
    asked: Asked,
    nested: Arc<Mutex<Option<CommandResult>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Echo {
    fn chat_recipient(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        self.asked.lock().unwrap().push((
            event.sender,
            event.sender_name.to_owned(),
            event.message.to_owned(),
            event.recipient,
        ));
        if event.message == "outer" && event.recipient == 13 {
            let result = host
                .commands()
                .run_command_with_output("execute run say nested");
            *self.nested.lock().unwrap() = Some(result);
        }
        if event.message == "nested" && event.recipient == 13 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Commands run by a listener must not contribute to the block's result callback.
struct Queries(Arc<Mutex<Vec<bool>>>);

#[mod_api::__subscriptions]
impl Hooks for Queries {
    fn chat_recipient(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ChatRecipientEvent<'_>,
    ) -> Flow {
        if event.recipient == 12 {
            let console = host.commands().run_command("time query gametime");
            let player = host.players().player_by_name("guest12").unwrap();
            let entity = player.entity().run_command(
                host,
                "time query gametime",
                PermissionLevel::from_level(2),
                false,
            );
            self.0.lock().unwrap().extend([console, entity]);
        }
        Flow::Continue
    }
}

/// A placed operator guest, named `guest<conn>`, and what the hub sends it.
fn join_guest(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid: u128::from(conn),
            name: format!("guest{conn}"),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        conn as i32,
        [0.5, 64.0, 0.5],
        2,
    );
    guest.placed = true;
    guest.permission = 4;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    rx
}

/// Guests 12 (the sender) and 13 on a LAN host, with what each is sent.
fn two_guests(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
) -> (mpsc::Receiver<Out>, mpsc::Receiver<Out>) {
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        ..NetConfig::lan(0, String::new(), 3)
    };
    let config = sunrise_sim::lan::hub::HubConfig {
        default_game_mode: 0,
        guest_commands: true,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 3,
        integrated: true,
        levels: game.server.level_shapes.clone(),
        level_seeds: game.server.levels.0.runtime.seeds(),
    };
    let settings = sunrise_gameplay::game::server_side::lan::LanSettings {
        port: 0,
        guest_commands: true,
        force_game_mode: false,
    };
    game.lan_host(settings, net, config).unwrap();
    (join_guest(game, 12), join_guest(game, 13))
}

fn packets(rx: &mpsc::Receiver<Out>) -> impl Iterator<Item = play::Clientbound> + '_ {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = sunrise_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            play::Clientbound::decode(id, reader.rest()).ok()
        })
}

/// How many public chat lines (signed or disguised) a guest was sent since the last call.
fn lines(rx: &mpsc::Receiver<Out>) -> usize {
    packets(rx)
        .filter(|packet| {
            matches!(
                packet,
                play::Clientbound::PlayerChat(_) | play::Clientbound::DisguisedChat(_)
            )
        })
        .count()
}

fn signed(content: &str) -> ChatMessage {
    ChatMessage {
        sender: 12,
        index: 0,
        signature: Some([9; sunrise_protocol::secure_chat::SIGNATURE_BYTES]),
        content: content.to_owned(),
        timestamp: 0,
        salt: 0,
        last_seen: Vec::new(),
        unsigned_content: None,
    }
}

/// A player's line: asked once per receiver, the sender included, with the text players see; the
/// hidden receiver is sent nothing and the others still are; a line nobody hides reaches all.
#[test]
#[ignore = "daily"]
fn a_chat_line_hidden_from_one_receiver_reaches_the_others() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            let bound = play::ChatTypeBound {
                chat_type: 0,
                name: sunrise_sim::lan::data::component_nbt(&Text::literal("guest12").component()),
                target: None,
            };
            let say = |game: &mut sunrise_gameplay::game::ServerGame<'_>, content: &str| {
                game.lan_broadcast_chat(
                    (12, "guest12"),
                    content,
                    Text::literal(format!("<guest12> {content}")),
                    &bound,
                    sunrise_gameplay::game::server_side::lan_chat::ChatDelivery::Player(&signed(
                        content,
                    )),
                );
            };
            say(game, "a secret");
            let play::Clientbound::PlayerChat(chat) = packets(&sender).next().unwrap() else {
                panic!("the sender still sees signed player chat");
            };
            assert_eq!(chat.content, "a secret");
            assert_eq!(
                chat.signature.as_deref(),
                Some([9; sunrise_protocol::secure_chat::SIGNATURE_BYTES].as_slice())
            );
            assert_eq!(lines(&other), 0, "the hidden receiver is not sent it");
            assert_eq!(
                std::mem::take(&mut *asked.lock().unwrap()),
                [
                    (12, "guest12".to_owned(), "a secret".to_owned(), 12),
                    (12, "guest12".to_owned(), "a secret".to_owned(), 13),
                ],
                "asked once per receiver, the sender included"
            );
            say(game, "hello");
            assert_eq!(lines(&sender), 1);
            let play::Clientbound::PlayerChat(chat) = packets(&other).next().unwrap() else {
                panic!("a line no mod hides reaches everyone");
            };
            assert_eq!(chat.global_index, 0, "an omitted line consumes no index");
        });
    });
}

/// `/say` and `/me` are public too, signed or not: the hidden receiver is not sent them either,
/// and `/msg` is not asked about at all.
#[test]
#[ignore = "daily"]
fn say_and_me_are_asked_per_receiver_and_a_whisper_is_not() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (sender, other) = two_guests(game);
            lines(&sender);
            lines(&other);
            assert!(game.guest_message_command(12, "say a secret", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 0, "/say, unsigned: hidden from guest 13");
            assert!(game.guest_message_command(12, "me waves", None));
            assert_eq!(lines(&sender), 1);
            assert_eq!(lines(&other), 1, "/me: nothing hides it");
            assert_eq!(asked.lock().unwrap().len(), 4, "two lines, two receivers");
            asked.lock().unwrap().clear();
            assert!(game.guest_message_command(12, "msg guest13 a secret", None));
            assert!(asked.lock().unwrap().is_empty(), "a whisper is not public");
            assert_eq!(lines(&other), 1, "and it reaches its target");
        });
    });
}

fn place_say_block(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> BlockPos {
    game.build_registries();
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.as_console(|game| game.run_command(
        r#"setblock 5 100 2 command_block[facing=up]{Command:"say block secret",CustomName:"Announcer",conditionMet:1b}"#,
    )));
    BlockPos::new(5, 100, 2)
}

#[test]
#[ignore = "daily"]
fn console_and_command_block_say_can_be_hidden_per_receiver() {
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (first, second) = two_guests(game);
            let block = place_say_block(game);
            let level = game.level(game.dim.id()).unwrap();
            game.fake_player(level, 14, "robot").unwrap();
            lines(&first);
            lines(&second);

            assert!(game.as_console_in_spawn_level(|game| game.run_command("say console secret")));
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0);
            game.command_block_tick(block);
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.info.chat_visibility = 1; // Commands Only
                });
            assert!(
                game.as_console_in_spawn_level(|game| game.run_command("say public announcement"))
            );
            assert_eq!(lines(&first), 1);
            assert_eq!(lines(&second), 0, "Commands Only does not show /say");
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    ("Server", "console secret", 12),
                    ("Server", "console secret", 13),
                    ("Announcer", "block secret", 12),
                    ("Announcer", "block secret", 13),
                    ("Server", "public announcement", 12),
                ]
                .map(|(name, message, receiver)| (
                    0,
                    name.to_owned(),
                    message.to_owned(),
                    receiver
                )),
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn dispatcher_say_uses_player_chat_for_the_source_too() {
    use play::Clientbound as C;
    use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
    use sunrise_gameplay_common::game::lan_actions::GuestCommand;

    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "ignorer",
        Box::new(Ignorer(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (peer, actor) = two_guests(game);
            let chat = |rx| {
                packets(rx)
                    .filter_map(|packet| match packet {
                        C::DisguisedChat(p) => Some((false, p.message)),
                        C::SystemChat(p) => Some((true, p.content)),
                        _ => None,
                    })
                    .map(|(system, tag)| (system, comp_text(&tag_to_json(&tag)).plain()))
                    .collect::<Vec<_>>()
            };
            chat(&peer);
            chat(&actor);
            for (message, visibility, delivered) in [
                ("a secret", 0, false),
                ("hello", 0, true),
                ("commands only", 1, false),
            ] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| {
                        guest.info.chat_visibility = visibility;
                    });
                assert!(game.run_guest_command(GuestCommand {
                    conn: 13,
                    command: format!("execute run say {message}"),
                    permission: 4,
                    integrated: true,
                    quiet: false,
                }));
                assert_eq!(chat(&peer), [(false, message.to_owned())]);
                let actor_lines = chat(&actor);
                if delivered {
                    assert_eq!(actor_lines, [(false, message.to_owned())]);
                } else {
                    assert!(actor_lines.is_empty(), "no indirect copy: {actor_lines:?}");
                }
            }
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    ("a secret", 12),
                    ("a secret", 13),
                    ("hello", 12),
                    ("hello", 13),
                    ("commands only", 12),
                ]
                .map(|(message, recipient)| (
                    13,
                    "guest13".to_owned(),
                    message.to_owned(),
                    recipient
                )),
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn listener_commands_do_not_count_as_command_block_successes() {
    let name = "cut_layout::game::server_side::native_chat_recipients_tests::listener_commands_do_not_count_as_command_block_successes";
    sunrise_gameplay_common::game::in_own_process(name, || {
        let results = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "queries",
            Box::new(Queries(results.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _receivers = two_guests(game);
                game.reset_entity_residency();
                let pos = place_say_block(game);
                game.command_block_tick(pos);
                assert_eq!(
                    *results.lock().unwrap(),
                    [true, true],
                    "both listener queries ran"
                );
                let sunrise_level::containers::BlockEntity::CommandBlock(block) =
                    &game.world.block_entities[&(pos.x, pos.y, pos.z)]
                else {
                    panic!("expected the command block");
                };
                assert_eq!(block.success_count, 1, "only the block's /say counts");
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn nested_say_passes_through_the_same_recipient_checks() {
    let asked = Asked::default();
    let nested = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "echo",
        Box::new(Echo {
            asked: asked.clone(),
            nested: nested.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (first, second) = two_guests(game);
            lines(&first);
            lines(&second);
            assert!(game.as_console_in_spawn_level(|game| game.run_command("say outer")));
            let result = nested.lock().unwrap().take().unwrap();
            assert!(result.success, "nested /say is a normal command");
            assert!(
                result.feedback.is_empty(),
                "the public message is not command feedback"
            );
            use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
            let disguised = |rx| {
                packets(rx)
                    .filter_map(|packet| match packet {
                        play::Clientbound::DisguisedChat(chat) => {
                            Some(comp_text(&tag_to_json(&chat.message)).plain())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                disguised(&first),
                ["outer", "nested"],
                "the first delivery already happened when the second listener runs"
            );
            assert_eq!(
                disguised(&second),
                ["outer"],
                "outer reaches guest 13 while the nested veto still applies"
            );
            let seen = asked.lock().unwrap();
            assert_eq!(
                seen.iter()
                    .map(|row| (row.2.as_str(), row.3))
                    .collect::<Vec<_>>(),
                [("outer", 12), ("outer", 13), ("nested", 12), ("nested", 13)],
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn panicking_chat_filter_is_reported_without_losing_other_vetoes() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_chat_recipients_tests::panicking_chat_filter_is_reported_without_losing_other_vetoes",
        "SUNRISE_CHAT_FILTER_PANIC_CASE",
        &["panic"],
    ) {
        return;
    }
    struct Broken;
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn chat_recipient(&self, _: &mut dyn ServerHost, _: &mut ChatRecipientEvent<'_>) -> Flow {
            panic!("chat filter failed");
        }
    }
    let mods = vec![
        mod_api::ModEntry::new("a_broken", Box::new(Broken)),
        mod_api::ModEntry::new("z_ignorer", Box::new(Ignorer(Asked::default()))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (first, second) = two_guests(game);
            lines(&first);
            lines(&second);
            assert!(game.as_console_in_spawn_level(|game| game.run_command("say first secret")));
            assert!(game.as_console_in_spawn_level(|game| game.run_command("say second secret")));
            assert_eq!(lines(&first), 2);
            assert_eq!(lines(&second), 0);
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "a_broken",
                    hook: "chat_recipient",
                }]
            );
        });
    });
}

/// MessageArgument uses the command source's permission and selectors; its result is unsigned display only.
#[test]
#[ignore = "daily"]
fn guest_public_command_selectors_change_display_not_the_signed_body() {
    use play::Clientbound as C;
    use sunrise_game_data::scoreboard::{comp_text, tag_to_json};
    let asked = Asked::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "chat_selectors",
            Box::new(Ignorer(asked.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let (first, second) = two_guests(game);
                lines(&first);
                lines(&second);
                let plain = |tag| comp_text(&tag_to_json(&tag)).plain();
                let body = "hello @s";
                let signed_message = signed(body);
                assert!(game.guest_message_command(
                    12,
                    "say hello @s",
                    Some(&vec![("message".into(), signed_message.clone())])
                ));
                for receiver in [&first, &second] {
                    let C::PlayerChat(chat) = packets(receiver)
                        .find(|p| matches!(p, C::PlayerChat(_)))
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    assert_eq!(
                        chat.signature.as_deref(),
                        signed_message
                            .signature
                            .as_ref()
                            .map(|signature| signature.as_slice())
                    );
                    assert_eq!(chat.content, body);
                    assert_eq!(plain(chat.unsigned_content.unwrap()), "hello guest12");
                }
                assert_eq!(asked.lock().unwrap().len(), 2);
                assert!(
                    asked
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|row| row.2 == "hello guest12")
                );
                asked.lock().unwrap().clear();
                assert!(game.guest_message_command(12, "me waves to @s", None));
                let C::DisguisedChat(chat) = packets(&second)
                    .find(|p| matches!(p, C::DisguisedChat(_)))
                    .unwrap()
                else {
                    unreachable!()
                };
                assert_eq!(plain(chat.message), "waves to guest12");
                assert_eq!(asked.lock().unwrap().len(), 2);
                assert!(
                    asked
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|row| row.2 == "waves to guest12")
                );
                asked.lock().unwrap().clear();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.permission = 0);
                assert!(game.guest_message_command(12, "me waves to @s", None));
                let C::DisguisedChat(chat) = packets(&second)
                    .find(|p| matches!(p, C::DisguisedChat(_)))
                    .unwrap()
                else {
                    unreachable!()
                };
                assert_eq!(plain(chat.message), "waves to @s");
                assert_eq!(asked.lock().unwrap().len(), 2);
                assert!(
                    asked
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|row| row.2 == "waves to @s")
                );
            });
        },
    );
}

/// Two lines that each make the listener say the other would reenter the broadcast without
/// end: each nested line waits for the running one, and a line repeating one that led to it
/// is dropped, so the exchange ends after each was said once.
#[test]
#[ignore = "daily"]
fn a_relay_answering_its_own_answer_stops_at_the_repeat() {
    struct Relay(Asked);
    #[mod_api::__subscriptions]
    impl Hooks for Relay {
        fn chat_recipient(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ChatRecipientEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push((
                event.sender,
                event.sender_name.to_owned(),
                event.message.to_owned(),
                event.recipient,
            ));
            if event.recipient == 13 {
                let answer = match event.message {
                    "ping" => "execute run say pong",
                    "pong" => "execute run say ping",
                    _ => return Flow::Continue,
                };
                host.commands().run_command_with_output(answer);
            }
            Flow::Continue
        }
    }
    let asked = Asked::default();
    let mods = vec![mod_api::ModEntry::new(
        "relay",
        Box::new(Relay(asked.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            two_guests(game);
            assert!(game.as_console_in_spawn_level(|game| game.run_command("say ping")));
            let seen: Vec<_> = asked
                .lock()
                .unwrap()
                .iter()
                .map(|row| (row.2.clone(), row.3))
                .collect();
            assert_eq!(
                seen,
                [
                    ("ping".to_owned(), 12),
                    ("ping".to_owned(), 13),
                    ("pong".to_owned(), 12),
                    ("pong".to_owned(), 13),
                ]
            );
        });
    });
}

/// `/teammsg` is chat to the team: the other member is sent a disguised chat of the world's
/// `team_msg_command_incoming` type, the sender one of `team_msg_command_outgoing`, both
/// bound to the team's display name; a player outside the team is sent nothing.
#[test]
#[ignore = "daily"]
fn teammsg_sends_the_team_chat_types_bound_to_the_team_name() {
    with_game(|game| {
        let (sender, other) = two_guests(game);
        let outsider = join_guest(game, 14);
        lines(&sender);
        lines(&other);
        game.score.board.add_team("red");
        game.score.board.add_to_team("guest12", "red");
        game.score.board.add_to_team("guest13", "red");
        game.guest_command(12, "teammsg hi", None);
        game.run_guest_commands();
        let hub = &game.server.lan.as_ref().unwrap().hub;
        let (outgoing, incoming) = (
            hub.chat_type_id("team_msg_command_outgoing")
                .unwrap()
                .network_id() as i32,
            hub.chat_type_id("team_msg_command_incoming")
                .unwrap()
                .network_id() as i32,
        );
        assert_ne!(outgoing, incoming);
        let hi = sunrise_sim::lan::data::component_nbt(&Text::literal("hi").component());
        let play::Clientbound::DisguisedChat(mine) = packets(&sender).next().unwrap() else {
            panic!("the sender is sent chat");
        };
        assert_eq!(mine.chat_type.chat_type, outgoing);
        assert_eq!(mine.message, hi);
        let play::Clientbound::DisguisedChat(theirs) = packets(&other).next().unwrap() else {
            panic!("the team member is sent chat");
        };
        assert_eq!(theirs.chat_type.chat_type, incoming);
        assert_eq!(theirs.message, hi);
        assert!(
            theirs.chat_type.target.is_some(),
            "bound to the team's name"
        );
        assert_eq!(theirs.chat_type.target, mine.chat_type.target);
        assert_eq!(
            lines(&outsider),
            0,
            "a player outside the team is sent nothing"
        );
    });
}

/// The vanilla pack is not required (`/datapack disable vanilla`) and a `pack.mcmeta` filter can
/// hide a built-in `chat_type` file, so the registry may lack a chat type. `ChatType.bind` then
/// throws, `Commands.performCommand` catches it and the command fails with `command.failed`:
/// nobody is sent the chat, and nothing panics.
#[test]
#[ignore = "daily"]
fn teammsg_fails_without_a_panic_when_a_chat_type_is_missing() {
    with_game(|game| {
        let (sender, other) = two_guests(game);
        lines(&sender);
        lines(&other);
        game.score.board.add_team("red");
        game.score.board.add_to_team("guest12", "red");
        game.score.board.add_to_team("guest13", "red");
        let mut sync = sunrise_sim::lan::data::SyncData::empty();
        sync.dynamic
            .registries
            .insert("minecraft:chat_type".to_owned(), Vec::new());
        game.server.lan.as_mut().unwrap().hub.sync = Arc::new(sync);
        game.guest_command(12, "teammsg hi", None);
        game.run_guest_commands();
        assert_eq!(lines(&sender), 0);
        assert_eq!(lines(&other), 0);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
