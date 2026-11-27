//! A mod's command as a player's client and the server see it: bounded numbers on the wire and
//! refused out of range, and a usage merged into a vanilla command.

use std::sync::{Arc, Mutex};
use sunrise_game_data::text;
use sunrise_protocol::{
    buf::Reader,
    ids::play::clientbound::COMMANDS,
    packets::play::{Clientbound, Commands},
};
use sunrise_sim::lan::{Out, PlayerSender};
use {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::game::lan_actions::GuestCommand,
};

/// What the executors received, in order: the usage's first word and its argument.
type Calls = Arc<Mutex<Vec<(String, Option<mod_api::CommandArgumentValue>)>>>;

struct ShapedCommands(Calls);

#[mod_api::__subscriptions]
impl mod_api::Hooks for ShapedCommands {
    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        for (usage, argument, result) in [
            ("pay <amount:integer(0,1000)>", "amount", 1),
            ("bank <total:long(-5,9000000000)>", "total", 1),
            ("time season <name:word>", "name", 1),
            ("seed", "", 7),
        ] {
            let calls = self.0.clone();
            event
                .commands
                .push(mod_api::Command::new(usage).executes(move |_, _, command| {
                    calls.lock().unwrap().push((
                        command.literals[0].clone(),
                        command.argument(argument).cloned(),
                    ));
                    Ok(result)
                }));
        }
    }
}

/// A server with the shaped commands and its two guests, guest 12 (level 2) receiving its packets
/// on the returned channel from the tree it is sent when the commands register.
fn with_shaped_commands(
    test: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        &Calls,
        std::sync::mpsc::Receiver<Out>,
    ),
) {
    let calls = Calls::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "shape_test",
            Box::new(ShapedCommands(calls.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.build_registries();
                let (tx, rx) = std::sync::mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                        guest.permission = 2;
                    });
                game.register_native_commands();
                test(game, &calls, rx);
            });
        },
    );
}

/// The last command tree in what the guest received.
fn sent_tree(rx: &std::sync::mpsc::Receiver<Out>) -> Commands {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            if reader.varint().unwrap() != COMMANDS {
                return None;
            }
            match Clientbound::decode(COMMANDS, reader.rest()).unwrap() {
                Clientbound::Commands(tree) => Some(tree),
                _ => unreachable!(),
            }
        })
        .last()
        .expect("the guest was sent its command tree")
}

/// The children of the node at `path` from the root, by name.
fn children<'a>(tree: &'a Commands, path: &[&str]) -> Vec<&'a str> {
    let mut node = tree.root as usize;
    for name in path {
        node = tree.nodes[node]
            .children
            .iter()
            .map(|&c| c as usize)
            .find(|&c| tree.nodes[c].name == *name)
            .unwrap_or_else(|| panic!("no `{name}` in {path:?}"));
    }
    tree.nodes[node]
        .children
        .iter()
        .map(|&c| tree.nodes[c as usize].name.as_str())
        .collect()
}

fn command(line: &str, permission: u8) -> GuestCommand {
    GuestCommand {
        conn: 12,
        command: line.into(),
        permission,
        integrated: true,
        quiet: false,
    }
}

#[test]
#[ignore = "daily"]
fn bounded_numbers_reach_the_client_and_refuse_out_of_range() {
    in_own_process(
        "cut_layout::game::native_server::command_shape_tests::bounded_numbers_reach_the_client_and_refuse_out_of_range",
        || {
            with_shaped_commands(|game, calls, rx| {
                let tree = sent_tree(&rx);
                let parser = |name: &str| {
                    tree.nodes
                        .iter()
                        .find(|n| n.name == name)
                        .and_then(|n| n.parser.clone())
                        .unwrap()
                };
                // `brigadier:integer` and `brigadier:long` (protocol ids 3 and 4 of
                // `minecraft:command_argument_type`), then `serializeToNetwork`.
                assert_eq!(parser("amount"), (3, vec![3, 0, 0, 0, 0, 0, 0, 0x03, 0xE8]));
                assert_eq!(
                    parser("total"),
                    (
                        4,
                        vec![
                            3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFB, 0, 0, 0, 0x02, 0x18,
                            0x71, 0x1A, 0x00
                        ]
                    )
                );

                for (line, refusal) in [
                    (
                        "pay 1001",
                        text::Text::tr("argument.integer.big", &[&1000, &1001]),
                    ),
                    ("pay -1", text::Text::tr("argument.integer.low", &[&0, &-1])),
                    (
                        "bank 9000000001",
                        text::Text::tr(
                            "argument.long.big",
                            &[&9_000_000_000_i64, &9_000_000_001_i64],
                        ),
                    ),
                    ("bank -6", text::Text::tr("argument.long.low", &[&-5, &-6])),
                ] {
                    let (ok, chat) = game.run_guest_command_told(command(line, 2)).unwrap();
                    assert!(!ok, "{line}");
                    assert_eq!(chat[0].plain(), refusal.plain(), "{line}");
                    assert_eq!(chat[0].spans[0].style.color, Some(text::RED), "{line}");
                }
                assert!(calls.lock().unwrap().is_empty(), "no executor ran");

                for line in ["pay 1000", "bank 9000000000"] {
                    assert!(game.run_guest_command_told(command(line, 2)).unwrap().0);
                }
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        (
                            "pay".to_owned(),
                            Some(mod_api::CommandArgumentValue::Integer(1000))
                        ),
                        (
                            "bank".to_owned(),
                            Some(mod_api::CommandArgumentValue::Long(9_000_000_000))
                        ),
                    ]
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_usage_merged_into_a_vanilla_command_keeps_its_usages() {
    in_own_process(
        "cut_layout::game::native_server::command_shape_tests::a_usage_merged_into_a_vanilla_command_keeps_its_usages",
        || {
            with_shaped_commands(|game, calls, rx| {
                // The client gets one `/time`, vanilla's usages first and the mod's after them.
                let tree = sent_tree(&rx);
                let time = children(&tree, &["time"]);
                assert_eq!(time.last(), Some(&"season"), "{time:?}");
                for vanilla in ["set", "add", "query"] {
                    assert!(time.contains(&vanilla), "{vanilla} in {time:?}");
                }
                assert_eq!(
                    children(&tree, &[])
                        .iter()
                        .filter(|name| **name == "time")
                        .count(),
                    1
                );

                let (ok, chat) = game
                    .run_guest_command_told(command("time add 1", 2))
                    .unwrap();
                assert!(
                    ok,
                    "{:?}",
                    chat.iter().map(|line| line.plain()).collect::<Vec<_>>()
                );
                assert!(
                    calls.lock().unwrap().is_empty(),
                    "vanilla's usage stays vanilla's"
                );
                assert!(
                    game.run_guest_command_told(command("time season spring", 2))
                        .unwrap()
                        .0
                );
                // Vanilla's `/time` keeps its permission: a level-0 player does not reach the
                // mod's word under it.
                assert!(
                    !game
                        .run_guest_command_told(command("time season spring", 0))
                        .unwrap()
                        .0
                );
                // The later executor at vanilla's `seed` node runs instead of vanilla's.
                let ((ok, _), outcome) = game.recording_command_outcome(|game| {
                    game.run_guest_command_told(command("seed", 2)).unwrap()
                });
                assert!(ok);
                assert_eq!(outcome, Some((true, 7)));
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        (
                            "time".to_owned(),
                            Some(mod_api::CommandArgumentValue::String("spring".into()))
                        ),
                        ("seed".to_owned(), None),
                    ]
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
