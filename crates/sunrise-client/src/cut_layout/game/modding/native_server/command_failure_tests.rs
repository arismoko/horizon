use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_game_data::{gamerules, text};
use sunrise_protocol::{
    buf::Reader, ids::play::clientbound::SYSTEM_CHAT, packets::play::Clientbound,
};
use sunrise_sim::lan::{Out, PlayerSender};
use {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::game::lan_actions::GuestCommand,
};

pub(crate) struct FailingCommand(pub(crate) Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for FailingCommand {
    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        // Java's `source.sendFailure(text); return 0;`.
        event
            .commands
            .push(mod_api::Command::new("failzero").executes(|host, _, _| {
                host.commands()
                    .failure(mod_api::Text::literal("Not enough mana"))
                    .expect("a shallow line");
                Ok(0)
            }));
        let calls = self.0.clone();
        event.commands.push(
            mod_api::Command::new("rejecttest").executes(move |_, _, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                Err(mod_api::Text::literal("No room for another item").into())
            }),
        );
        for (usage, depth) in [
            ("rejectinvalid", mod_api::text::MAX_TEXT_DEPTH + 1),
            // Each parent contributes a compound and an `extra` list. The
            // leaf makes 511 containers; only the formatted failure exceeds 512.
            ("rejectformatted", mod_api::text::MAX_TEXT_DEPTH / 2),
        ] {
            event
                .commands
                .push(mod_api::Command::new(usage).executes(move |_, _, _| {
                    let mut message = mod_api::Text::literal("unserializable failure");
                    for _ in 1..depth {
                        message = mod_api::Text::literal("").append(message);
                    }
                    Err(message.into())
                }));
        }
        for (name, fits) in [("rejectsavededge", true), ("rejectsavedover", false)] {
            event
                .commands
                .push(mod_api::Command::new(name).executes(move |_, _, _| {
                    let mut leaf = mod_api::Text::literal("tracked failure");
                    if fits {
                        leaf.style.bold = Some(true);
                    }
                    // The timestamp and red parent add four containers. Each nested
                    // `extra` adds two. A styled leaf is a compound, an unstyled leaf
                    // is a string. Including the save's three containers: 512 or 513.
                    for _ in 0..if fits { 252 } else { 253 } {
                        leaf = mod_api::Text::literal("").append(leaf);
                    }
                    Err(leaf.into())
                }));
        }
        for (name, first) in [("rejectmetaedge", 32_762), ("rejectmetaover", 32_763)] {
            event
                .commands
                .push(mod_api::Command::new(name).executes(move |_, _, _| {
                    Err(mod_api::Text::literal("a".repeat(first))
                        .append(mod_api::Text::literal("b".repeat(32_762)))
                        .into())
                }));
        }
    }
}

pub(crate) fn with_failure_sources(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "failure_test",
            Box::new(FailingCommand(Arc::default())),
        )]
        .into(),
        || crate::game::native_server::tests::with_command_sources(test),
    );
}

#[test]
#[ignore = "daily"]
fn executor_failure_respects_source_silence_and_stores_zero() {
    in_own_process(
        "cut_layout::game::native_server::command_failure_tests::executor_failure_respects_source_silence_and_stores_zero",
        || {
            let calls = Arc::new(AtomicUsize::new(0));
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "failure_test",
                    Box::new(FailingCommand(calls.clone())),
                )]
                .into(),
                || {
                    with_guests(|game| {
                        game.build_registries();
                        game.register_native_commands();
                        assert!(
                            game.ui
                                .rules
                                .set("send_command_feedback", gamerules::Value::Bool(false),)
                        );
                        let command = |line: &str, quiet| GuestCommand {
                            conn: 12,
                            command: line.into(),
                            permission: 2,
                            integrated: true,
                            quiet,
                        };

                        let (ok, chat) = game
                            .run_guest_command_told(command("rejecttest", false))
                            .unwrap();
                        assert!(!ok);
                        assert_eq!(chat.len(), 1);
                        assert_eq!(chat[0].plain(), "No room for another item");
                        assert_eq!(chat[0].spans[0].style.color, Some(text::RED));
                        assert_eq!(calls.load(Ordering::Relaxed), 1);

                        let (ok, chat) = game
                            .run_guest_command_told(command("rejecttest", true))
                            .unwrap();
                        assert!(!ok);
                        assert!(chat.is_empty());
                        assert_eq!(calls.load(Ordering::Relaxed), 2);

                        // The guest route serializes these lines to SystemChat.
                        let ((ok, chat), outcome) = game.recording_command_outcome(|game| {
                            game.run_guest_command_told(command("rejectinvalid", false))
                                .unwrap()
                        });
                        assert!(!ok);
                        assert_eq!(outcome, Some((false, 0)));
                        assert_eq!(chat.len(), 1);
                        assert_eq!(
                            chat[0].plain(),
                            text::Text::tr("command.failed", &[]).plain()
                        );

                        // The original component fits on the wire; the red parent
                        // and its `extra` list put the formatted failure over the limit.
                        let (tx, rx) = std::sync::mpsc::channel();
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .update_guest(12, |guest| {
                                guest.sender = PlayerSender::new(tx, Arc::default());
                            });
                        let (ok, outcome) = game.recording_command_outcome(|game| {
                            game.run_guest_command(command("rejectformatted", false))
                        });
                        assert!(!ok);
                        assert_eq!(outcome, Some((false, 0)));
                        let received: Vec<_> = rx
                            .try_iter()
                            .flat_map(|out| match out {
                                Out::Packet(body) => vec![body],
                                Out::Packets(bodies) => bodies,
                                Out::Disconnect(_) => Vec::new(),
                            })
                            .filter_map(|body| {
                                let mut reader = Reader::new(&body);
                                if reader.varint().unwrap() != SYSTEM_CHAT {
                                    return None;
                                }
                                let Clientbound::SystemChat(chat) =
                                    Clientbound::decode(SYSTEM_CHAT, reader.rest()).unwrap()
                                else {
                                    unreachable!()
                                };
                                Some(
                                    sunrise_gameplay_common::game::remote::remote_text(
                                        &chat.content,
                                    )
                                    .plain(),
                                )
                            })
                            .collect();
                        assert_eq!(received, [text::Text::tr("command.failed", &[]).plain()]);

                        assert!(game.run_guest_command(command(
                            "data merge storage test:out {success:42,result:42}",
                            true,
                        )));
                        assert!(!game.run_guest_command(command(
                            "execute store success storage test:out success int 1 \
                             store result storage test:out result int 1 run rejecttest",
                            true,
                        )));
                        assert!(game.run_guest_command(command(
                            "execute if data storage test:out {success:0,result:0}",
                            true,
                        )));
                    });
                },
            );
        },
    );
}

/// `CommandSourceStack.sendFailure` then `return 0`: the red line reaches the source whatever
/// `sendCommandFeedback` says, a silent source gets nothing, and the command succeeds with
/// result 0, which `execute store result` stores, where a returned failure stores 0 for success
/// too.
#[test]
#[ignore = "daily"]
fn a_failure_line_then_zero_is_a_red_line_and_a_zero_result() {
    in_own_process(
        "cut_layout::game::native_server::command_failure_tests::a_failure_line_then_zero_is_a_red_line_and_a_zero_result",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "failure_test",
                    Box::new(FailingCommand(Arc::default())),
                )]
                .into(),
                || {
                    with_guests(|game| {
                        game.build_registries();
                        game.register_native_commands();
                        assert!(
                            game.ui
                                .rules
                                .set("send_command_feedback", gamerules::Value::Bool(false))
                        );
                        let command = |line: &str, quiet| GuestCommand {
                            conn: 12,
                            command: line.into(),
                            permission: 2,
                            integrated: true,
                            quiet,
                        };

                        let ((ok, chat), outcome) = game.recording_command_outcome(|game| {
                            game.run_guest_command_told(command("failzero", false))
                                .unwrap()
                        });
                        assert!(ok, "the command ran");
                        assert_eq!(outcome, Some((true, 0)));
                        assert_eq!(chat.len(), 1);
                        assert_eq!(chat[0].plain(), "Not enough mana");
                        assert_eq!(chat[0].spans[0].style.color, Some(text::RED));

                        let (ok, chat) = game
                            .run_guest_command_told(command("failzero", true))
                            .unwrap();
                        assert!(ok);
                        assert!(chat.is_empty(), "a silent source gets no line");

                        assert!(game.run_guest_command(command(
                            "data merge storage test:out {success:42,result:42}",
                            true,
                        )));
                        assert!(game.run_guest_command(command(
                            "execute store success storage test:out success int 1 \
                             store result storage test:out result int 1 run failzero",
                            true,
                        )));
                        assert!(game.run_guest_command(command(
                            "execute if data storage test:out {success:1,result:0}",
                            true,
                        )));
                    });
                },
            );
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn cart_failure_metadata_keeps_timestamp_and_literal_fragments() {
    use sunrise_core::nbt::Tag;
    use sunrise_protocol::{
        entity_data::{Value, serializer},
        ids::play::clientbound::SET_ENTITY_DATA,
        packets::play::SetEntityData,
    };
    use sunrise_sim::lan::entity::States;

    in_own_process(
        "cut_layout::game::native_server::command_failure_tests::cart_failure_metadata_keeps_timestamp_and_literal_fragments",
        || {
            with_failure_sources(|game| {
                // Eleven ASCII bytes of timestamp take the flattened output to
                // 65,535 and 65,536 modified UTF-8 bytes, respectively.
                for (command, first) in [("rejectmetaedge", 32_762), ("rejectmetaover", 32_763)] {
                    game.carts
                        .update(0, |cart| cart.command.set_command(command));
                    game.world.game_time += 1;
                    assert!(game.perform_cart_command(99));
                    let mut states = States::default();
                    game.lan_object_states(&mut states, false, 0);
                    let cart = states
                        .as_slice()
                        .iter()
                        .find(|state| state.id == 99)
                        .unwrap();
                    let body = Clientbound::SetEntityData(SetEntityData {
                        id: cart.id,
                        values: cart.meta.clone(),
                    })
                    .encode();
                    let mut reader = Reader::new(&body);
                    assert_eq!(reader.varint().unwrap(), SET_ENTITY_DATA);
                    let Clientbound::SetEntityData(meta) =
                        Clientbound::decode(SET_ENTITY_DATA, reader.rest()).unwrap()
                    else {
                        unreachable!();
                    };
                    assert_eq!(meta.id, 99);
                    let (_, kind, Value::Text(output)) = meta
                        .values
                        .iter()
                        .find(|(index, _, _)| *index == 14)
                        .unwrap()
                    else {
                        panic!("cart output is not a component");
                    };
                    assert_eq!(*kind, serializer::COMPONENT);
                    let Some(Tag::String(prefix)) = output.get("text") else {
                        panic!("cart output does not preserve the timestamp component");
                    };
                    let prefix = prefix.to_string_lossy();
                    assert_eq!(prefix.len(), 11);
                    assert!(chrono::NaiveTime::parse_from_str(&prefix, "[%H:%M:%S] ").is_ok());
                    let decoded = sunrise_gameplay_common::game::remote::remote_text(output);
                    assert_eq!(
                        decoded.plain(),
                        format!("{prefix}{}{}", "a".repeat(first), "b".repeat(32_762))
                    );
                    assert!(
                        decoded
                            .spans
                            .iter()
                            .any(|span| span.style.color == Some(text::RED))
                    );
                }
            })
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
