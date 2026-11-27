//! A mod command's path follows the sender's permission everywhere it is visible: the command tree
//! a player receives keeps only the children they may run, the server's completions answer the same
//! test, and a registered argument type is asked with the sender, so a provider that completes a
//! mod's own registry offers only what that sender may use (#4307).

use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_command::{MOD_COMMAND, Registries};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_gameplay::game::lan::LanSettings;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    buf::{Codec, Reader},
    ids,
    packets::{configuration::ClientInformation, play},
};
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
use {
    sunrise_gameplay_common::game::native_server::CommandArguments,
    sunrise_gameplay_common::game::native_server::command_tree,
    sunrise_gameplay_common::game::native_server::register_dispatcher,
};

/// The warps the test mod publishes, with the level each one is for: `spawn` for everyone,
/// `secret` for an owner.
const WARPS: [(&str, u8); 2] = [("spawn", 0), ("secret", 4)];

/// The test mod's argument type: it completes the warps this sender may use, which is its own
/// policy — the game keeps the answer as it comes.
struct WarpNames;

impl mod_api::CommandArgumentType for WarpNames {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        let permission = request
            .sender
            .as_ref()
            .map_or(0, |sender| sender.permission().level());
        WARPS
            .into_iter()
            .filter(|(_, level)| *level <= permission)
            .filter(|(name, _)| name.starts_with(request.prefix.as_str()))
            .map(|(name, _)| request.suggest(name))
            .collect()
    }
}

/// A hosted world with `name` joined, placed and at the world spawn, the way
/// `game/interface/scoreboard/source_holder_tests.rs` builds one.
fn game_with_guest<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    name: &str,
) -> (sunrise_gameplay::game::ServerGame<'a>, mpsc::Receiver<Out>) {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.build_registries();
    game.set_allow_commands(true);
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "native-completion-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 2,
            uuid: 331,
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [4.5, 64.0, 4.5],
        2,
    );
    guest.dimension = "minecraft:overworld".into();
    guest.placed = true;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    (game, rx)
}

/// The warps command in the server's dispatcher and in every player's tree: `/warp` for everyone,
/// `/warp go <name>` for everyone, `/warp admin <name>` for owners.
fn install_warps(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let arguments: CommandArguments = std::collections::HashMap::from([(
        "test:warp".to_owned(),
        Arc::new(WarpNames) as Arc<dyn mod_api::CommandArgumentType>,
    )]);
    let commands = [
        mod_api::Command::new("warp"),
        mod_api::Command::new("warp go <name:test:warp>"),
        mod_api::Command::op("warp admin <name:test:warp>", 4),
    ];
    let (tree, _, problems) =
        command_tree(&commands, &sunrise_command::Dispatcher::new(), &arguments);
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = (*game.ui.dispatcher.0).clone();
    register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
    game.ui.dispatcher.0 = Arc::new(dispatcher);
    let host = game.server.lan.as_mut().unwrap();
    host.hub.set_mod_commands(0, tree);
    game.send_player_command_tree(2);
}

/// The last command tree the guest was sent.
fn last_tree(rx: &mpsc::Receiver<Out>) -> play::Commands {
    let mut last = None;
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            if r.varint().unwrap() != ids::play::clientbound::COMMANDS {
                continue;
            }
            last = Some(play::Commands::read(&mut r).unwrap());
        }
    }
    last.expect("no command tree")
}

struct ModifierId;

impl mod_api::CommandArgumentType for ModifierId {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Id
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(format!("parsed {input}"))
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        assert_eq!(request.prefix, "tconstruct:ha");
        vec![request.suggest("tconstruct:haste")]
    }
}

#[test]
#[ignore = "daily"]
fn custom_id_reads_a_mid_command_identifier_and_sends_its_vanilla_parser() {
    sunrise_gameplay_common::game::in_own_process(
        concat!(
            module_path!(),
            "::custom_id_reads_a_mid_command_identifier_and_sends_its_vanilla_parser"
        )
        .split_once("::")
        .unwrap()
        .1,
        || {
            use sunrise_command::{Arg, Dispatcher, network};
            let arguments = CommandArguments::from([(
                "test:modifier".to_owned(),
                Arc::new(ModifierId) as Arc<dyn mod_api::CommandArgumentType>,
            )]);
            let (tree, _, problems) = command_tree(
                &[mod_api::Command::new(
                    "modprobe <modifier:test:modifier> <amount:integer>",
                )],
                &Dispatcher::new(),
                &arguments,
            );
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
            let registries = Registries::default();
            for raw in ["tconstruct:haste", "group/haste"] {
                let line = format!("modprobe {raw} 2");
                let parsed = dispatcher.parse(&line, &registries);
                let (context, _) = parsed.executable().unwrap();
                assert_eq!(
                    context.get("modifier"),
                    Some(&Arg::Custom {
                        kind: "test:modifier".into(),
                        value: format!("parsed {raw}"),
                    })
                );
                assert_eq!(context.get("amount"), Some(&Arg::Int(2)));
            }
            for raw in ["tconstruct:haste#tag", "tconstruct:haste:extra"] {
                let line = format!("modprobe {raw} 2");
                assert!(dispatcher.parse(&line, &registries).executable().is_err());
            }

            let mut resources = None;
            let (mut game, rx) = game_with_guest(&mut resources, "IdTester");
            game.ui.dispatcher.0 = Arc::new(dispatcher);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .set_mod_commands(0, tree);
            game.send_player_command_tree(2);
            let packet = last_tree(&rx);
            let root = packet
                .nodes
                .iter()
                .find(|node| node.name == "modprobe")
                .unwrap();
            let modifier = &packet.nodes[root.children[0] as usize];
            // 26.3 ArgumentTypeInfos: resource_location is parser 36 and has no properties.
            assert_eq!(modifier.parser, Some((36, Vec::new())));
            assert_eq!(
                modifier.suggestions.as_deref(),
                Some("minecraft:ask_server")
            );
            let vanilla_client = network::from_packet(&packet, &Dispatcher::new());
            assert!(
                vanilla_client
                    .parse("modprobe tconstruct:haste 2", &registries)
                    .executable()
                    .is_ok()
            );
            assert_eq!(
                suggestions(&mut game, &rx, "/modprobe tconstruct:ha"),
                ["tconstruct:haste"]
            );
        },
    );
}

/// The children of the node named `name`, in the tree's order.
fn children(packet: &play::Commands, name: &str) -> Vec<String> {
    let node = packet
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("no `{name}` in the tree"));
    node.children
        .iter()
        .map(|&child| packet.nodes[child as usize].name.clone())
        .collect()
}

/// The completions the server answered the guest's request for `command` with.
fn suggestions(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    command: &str,
) -> Vec<String> {
    let mut w = Writer::new();
    w.varint(7);
    w.string(command);
    game.guest_suggestions(2, &w.data);
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            if r.varint().unwrap() != ids::play::clientbound::COMMAND_SUGGESTIONS {
                continue;
            }
            let _id = r.varint().unwrap();
            let _start = r.varint().unwrap();
            let _length = r.varint().unwrap();
            let count = r.count(usize::MAX).unwrap();
            let mut names = Vec::new();
            for _ in 0..count {
                names.push(r.string(32767).unwrap());
                if r.bool().unwrap() {
                    let _ = r.nbt().unwrap();
                }
            }
            return names;
        }
    }
    panic!("no command suggestions packet for `{command}`");
}

struct OversizedAndMany {
    mixed: bool,
}

impl mod_api::CommandArgumentType for OversizedAndMany {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(input.to_owned())
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        if self.mixed && !request.prefix.is_empty() {
            return vec![
                request.suggest("short"),
                request.suggest_from(request.prefix.len() - 1, "b".repeat(766)),
                request.suggest_from(request.prefix.len() - 1, "b".repeat(767)),
            ];
        }
        if self.mixed {
            return vec![
                request.suggest("a".repeat(32768)),
                request.suggest_with_tooltip("bad_tip", "b".repeat(65536)),
                request.suggest_with_tooltip("bad_nul", "\0".repeat(32768)),
                request.suggest_with_tooltip("ok_sibling", "valid tooltip"),
                request.suggest_with_tooltip("z".repeat(32767), "y".repeat(65535)),
            ];
        }
        // The whole result sorts before the server's packet cap.
        (0..1050)
            .rev()
            .map(|i| request.suggest(format!("cand_{i:04}")))
            .chain(std::iter::once(request.suggest("a".repeat(32768))))
            .collect()
    }
}

#[test]
#[ignore = "daily"]
fn oversized_entries_drop_individually_and_large_regular_replies_sort_before_truncation() {
    let mut resources = None;
    let (mut game, rx) = game_with_guest(&mut resources, "Alex");
    let arguments: CommandArguments = std::collections::HashMap::from([
        (
            "test:mixed".to_owned(),
            Arc::new(OversizedAndMany { mixed: true }) as Arc<dyn mod_api::CommandArgumentType>,
        ),
        (
            "test:many".to_owned(),
            Arc::new(OversizedAndMany { mixed: false }) as Arc<dyn mod_api::CommandArgumentType>,
        ),
    ]);
    let (tree, _, problems) = command_tree(
        &[
            mod_api::Command::new("mixed <value:test:mixed>"),
            mod_api::Command::new("probe <value:test:many>"),
        ],
        &sunrise_command::Dispatcher::new(),
        &arguments,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = (*game.ui.dispatcher.0).clone();
    register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
    game.ui.dispatcher.0 = Arc::new(dispatcher);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .set_mod_commands(0, tree);
    game.send_player_command_tree(2);
    let packet = last_tree(&rx);
    assert_eq!(children(&packet, "mixed"), ["value"]);
    assert_eq!(children(&packet, "probe"), ["value"]);

    assert_eq!(
        suggestions(&mut game, &rx, "mixed "),
        ["ok_sibling".to_owned(), "z".repeat(32767)]
    );
    let dispatcher = &game.ui.dispatcher.0;
    let registries = &game.ui.registries;
    let (_, local) =
        dispatcher.client_suggestions(&dispatcher.parse("mixed ", registries), 6, registries);
    assert_eq!(local.len(), 5);
    assert!(local.iter().any(|entry| entry.text == "a".repeat(32768)));
    assert!(local.iter().any(|entry| entry.text == "bad_tip"));
    assert!(local.iter().any(|entry| entry.text == "bad_nul"));

    let prefix = "a".repeat(32001);
    assert_eq!(
        suggestions(&mut game, &rx, &format!("mixed {prefix}a")),
        [format!("{prefix}{}", "b".repeat(766)), "short".into()]
    );

    let truncated = suggestions(&mut game, &rx, "probe ");
    assert_eq!(truncated.len(), 1000);
    assert_eq!(truncated[0], "cand_0000");
    assert_eq!(truncated[999], "cand_0999");
}

struct IdentifierCompletion;

#[mod_api::__subscriptions]
impl mod_api::Hooks for IdentifierCompletion {
    fn tab_complete(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::TabCompleteEvent<'_, '_>,
    ) {
        if event.command == "probe ex" {
            assert_eq!(event.range, [6, 8]);
            event
                .suggestions
                .push(mod_api::Suggestion::new((6, 8), "example:stone"))
                .unwrap();
        }
    }
}

#[test]
#[ignore = "daily"]
fn an_identifier_listener_replies_on_the_wire_and_leaves_room_for_the_next_argument() {
    let mods = vec![mod_api::ModEntry::new(
        "identifier_completion",
        Box::new(IdentifierCompletion),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut resources = None;
        let (mut game, rx) = game_with_guest(&mut resources, "Alex");
        let arguments = CommandArguments::new();
        let (tree, _, problems) = command_tree(
            &[mod_api::Command::new("probe <value:id> <count:integer>")],
            &sunrise_command::Dispatcher::new(),
            &arguments,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let mut dispatcher = (*game.ui.dispatcher.0).clone();
        register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
        game.ui.dispatcher.0 = Arc::new(dispatcher);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .set_mod_commands(0, tree);
        game.send_player_command_tree(2);
        last_tree(&rx);

        let typed = "probe ex";
        let mut request = Writer::new();
        request.varint(7);
        request.string(typed);
        game.guest_suggestions(2, &request.data);
        let mut answer = None;
        for out in rx.try_iter() {
            let bodies = match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut reader = Reader::new(&body);
                if reader.varint().unwrap() != ids::play::clientbound::COMMAND_SUGGESTIONS {
                    continue;
                }
                assert_eq!(reader.varint().unwrap(), 7);
                let start = reader.varint().unwrap();
                let length = reader.varint().unwrap();
                assert_eq!(reader.varint().unwrap(), 1);
                let text = reader.string(32767).unwrap();
                assert!(!reader.bool().unwrap());
                assert!(answer.replace((start, length, text)).is_none());
            }
        }
        let (start, length, text) = answer.expect("no suggestion packet");
        assert_eq!((start, length, text.as_str()), (6, 2, "example:stone"));
        let mut applied = typed.to_owned();
        applied.replace_range(start as usize..(start + length) as usize, &text);
        assert_eq!(applied, "probe example:stone");
        let completed = format!("{applied} 3");
        let parsed = game.ui.dispatcher.0.parse(&completed, &game.ui.registries);
        let (context, _) = parsed.executable().unwrap();
        assert_eq!(
            context.get("value"),
            Some(&sunrise_command::Arg::Str("example:stone".into()))
        );
    });
}

/// FTB Quests' `QuestObjectArgument`: a hex id, or a quoted `"#tag"` or title, with more
/// arguments after it. Its titles complete as insertable syntax, quoted.
struct QuestObject;

/// The quest titles the test mod knows.
const QUEST_TITLES: [&str; 2] = ["Getting Started", "Getting Rich"];

impl mod_api::CommandArgumentType for QuestObject {
    fn kind(&self) -> mod_api::CommandArgumentKind {
        mod_api::CommandArgumentKind::Quotable
    }

    fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
        Ok(format!("object {input}"))
    }

    fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
        // Brigadier hands the typed suffix, opening quote included.
        assert_eq!(request.prefix, "\"Getting St");
        let typed = request.prefix.trim_start_matches('"');
        QUEST_TITLES
            .into_iter()
            .filter(|title| title.starts_with(typed))
            .map(|title| request.suggest(format!("\"{title}\"")))
            .collect()
    }
}

/// `<object:test:quest_object>` and `<title:string>` read a quoted phrase with an argument after
/// it, on the server and on a vanilla client that rebuilds the tree from the packet: both nodes
/// are `brigadier:string` mode 1 (`QUOTABLE_PHRASE`), the mod's asks the server, and its
/// completion replaces the half-typed phrase from its opening quote.
#[test]
#[ignore = "daily"]
fn a_quotable_argument_reads_a_quoted_phrase_mid_command_and_goes_out_as_a_quotable_string() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_command_completion_tests::a_quotable_argument_reads_a_quoted_phrase_mid_command_and_goes_out_as_a_quotable_string",
        || {
            use sunrise_command::{Arg, Dispatcher, network};
            let arguments = CommandArguments::from([(
                "test:quest_object".to_owned(),
                Arc::new(QuestObject) as Arc<dyn mod_api::CommandArgumentType>,
            )]);
            let (tree, _, problems) = command_tree(
                &[
                    mod_api::Command::new("quests open <object:test:quest_object> <count:integer>"),
                    mod_api::Command::new("quests rename <title:string> <count:integer>"),
                ],
                &Dispatcher::new(),
                &arguments,
            );
            assert!(problems.is_empty(), "{problems:?}");
            let mut dispatcher = Dispatcher::new();
            register_dispatcher(&mut dispatcher, &tree, MOD_COMMAND, &arguments);
            let registries = Registries::default();
            for (typed, decoded) in [
                (r##""#chapter_one""##, "#chapter_one"),
                ("1A2B3C4D5E6F7A8B", "1A2B3C4D5E6F7A8B"),
                (r#"'say "hi" \'now\''"#, r#"say "hi" 'now'"#),
                (r#""""#, ""),
            ] {
                let line = format!("quests open {typed} 2");
                let parsed = dispatcher.parse(&line, &registries);
                let (context, _) = parsed.executable().unwrap();
                assert_eq!(
                    context.get("object"),
                    Some(&Arg::Custom {
                        kind: "test:quest_object".into(),
                        value: format!("object {decoded}"),
                    }),
                    "{line}"
                );
                assert_eq!(context.get("count"), Some(&Arg::Int(2)), "{line}");
            }
            let line = r#"quests rename "Getting \"Started\"" 2"#;
            let parsed = dispatcher.parse(line, &registries);
            assert_eq!(
                parsed.executable().unwrap().0.get("title"),
                Some(&Arg::Str(r#"Getting "Started""#.into()))
            );
            let line = r#"quests open "Getting 2"#;
            let error = dispatcher
                .parse(line, &registries)
                .executable()
                .unwrap_err();
            assert_eq!(error.key, "parsing.quote.expected.end");
            assert_eq!(error.cursor, line.len());

            let mut resources = None;
            let (mut game, rx) = game_with_guest(&mut resources, "QuestTester");
            // Vanilla's en_us text for `readerExpectedEndOfQuote`, in the game's language.
            assert_eq!(
                sunrise_game_data::text::Text::tr("parsing.quote.expected.end", &[]).plain(),
                "Unclosed quoted string"
            );
            game.ui.dispatcher.0 = Arc::new(dispatcher);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .set_mod_commands(0, tree);
            game.send_player_command_tree(2);
            let packet = last_tree(&rx);
            let node = |parent: &str, child: &str| {
                let parent = packet
                    .nodes
                    .iter()
                    .find(|node| node.name == parent)
                    .unwrap();
                parent
                    .children
                    .iter()
                    .map(|&i| &packet.nodes[i as usize])
                    .find(|node| node.name == child)
                    .unwrap()
            };
            // 26.3 ArgumentTypeInfos: brigadier:string is parser 5; StringArgumentSerializer
            // writes QUOTABLE_PHRASE as 1.
            let object = node("open", "object");
            assert_eq!(object.parser, Some((5, vec![1])));
            assert_eq!(object.suggestions.as_deref(), Some("minecraft:ask_server"));
            let title = node("rename", "title");
            assert_eq!(title.parser, Some((5, vec![1])));
            assert_eq!(title.suggestions, None);
            let vanilla_client = network::from_packet(&packet, &Dispatcher::new());
            for line in [
                r##"quests open "#chapter_one" 2"##,
                r#"quests rename 'Getting "Started"' 2"#,
            ] {
                assert!(
                    vanilla_client.parse(line, &registries).executable().is_ok(),
                    "{line}"
                );
            }

            // The completion over the wire: it replaces from the opening quote, and the line it
            // makes parses on the server.
            let typed = r#"quests open "Getting St"#;
            let mut request = Writer::new();
            request.varint(7);
            request.string(typed);
            game.guest_suggestions(2, &request.data);
            let mut answer = None;
            for out in rx.try_iter() {
                let bodies = match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => panic!("unexpected disconnect"),
                };
                for body in bodies {
                    let mut reader = Reader::new(&body);
                    if reader.varint().unwrap() != ids::play::clientbound::COMMAND_SUGGESTIONS {
                        continue;
                    }
                    assert_eq!(reader.varint().unwrap(), 7);
                    let start = reader.varint().unwrap();
                    let length = reader.varint().unwrap();
                    assert_eq!(reader.varint().unwrap(), 1);
                    let text = reader.string(32767).unwrap();
                    assert!(!reader.bool().unwrap());
                    assert!(answer.replace((start, length, text)).is_none());
                }
            }
            let (start, length, text) = answer.expect("no suggestion packet");
            assert_eq!(
                (start, length, text.as_str()),
                (12, 11, r#""Getting Started""#)
            );
            let mut applied = typed.to_owned();
            applied.replace_range(start as usize..(start + length) as usize, &text);
            assert_eq!(applied, r#"quests open "Getting Started""#);
            let completed = format!("{applied} 1");
            let parsed = game.ui.dispatcher.0.parse(&completed, &game.ui.registries);
            assert_eq!(
                parsed.executable().unwrap().0.get("object"),
                Some(&Arg::Custom {
                    kind: "test:quest_object".into(),
                    value: "object Getting Started".into(),
                })
            );
        },
    );
}

/// The `warp` tree and completions an operator gets, and a plain player does not: the admin child
/// literal and the private warp are absent until the level arrives, and running `warp admin` is
/// refused for the same reason in both states.
#[test]
#[ignore = "daily"]
fn a_plain_players_tree_and_completions_leave_out_the_paths_they_may_not_run() {
    let mut resources = None;
    let (mut game, rx) = game_with_guest(&mut resources, "Alex");
    install_warps(&mut game);
    assert_eq!(children(&last_tree(&rx), "warp"), ["go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp "), ["go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp go "), ["spawn"]);
    let at_player = Registries {
        permission: 0,
        ..Registries::default()
    };
    assert!(
        game.ui
            .dispatcher
            .0
            .parse("warp admin secret", &at_player)
            .executable()
            .is_err(),
        "running the admin path is refused, as it always was"
    );

    // `/op`: the operator list changes and the hub sends the trees again.
    let uuid = game.server.lan.as_ref().unwrap().hub.guests[0].uuid;
    {
        let host = game.server.lan.as_mut().unwrap();
        host.hub.ops.insert(uuid, 4);
        game.resend_commands_filtered();
    }
    assert_eq!(
        game.server.lan.as_ref().unwrap().hub.guests[0].permission,
        4
    );
    assert_eq!(children(&last_tree(&rx), "warp"), ["admin", "go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp "), ["admin", "go"]);
    assert_eq!(suggestions(&mut game, &rx, "warp go "), ["secret", "spawn"]);
    let at_owner = Registries {
        permission: 4,
        ..Registries::default()
    };
    assert!(
        game.ui
            .dispatcher
            .0
            .parse("warp admin secret", &at_owner)
            .executable()
            .is_ok(),
        "with the level, the path parses and runs"
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
