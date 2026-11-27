#![cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::play::Clientbound as C;
#[cfg(test)]
#[path = "native_server/command_failure_tests_server.rs"]
pub(crate) mod command_failure_tests;

#[cfg(test)]
#[path = "native_server/selector_tests_server.rs"]
pub(crate) mod selector_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::game::native_server::tests::with_command_sources;

    #[test]
    #[ignore = "daily"]
    fn self_deop_sends_the_tree_for_the_permission_the_listener_reads() {
        use sunrise_gameplay::game::server_side::dedicated::Lists;
        use sunrise_gameplay_common::game::{
            execution_role::ExecutionRole, lan_actions::GuestCommand,
        };
        use sunrise_server_files::user_lists::{
            BanList, IpBanList, NameAndId, OpEntry, OpList, WhiteList,
        };
        use sunrise_sim::lan::PlayerSender;

        type Reads = Arc<Mutex<Vec<(Option<u8>, bool)>>>;
        struct Observe(Reads);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn player_commands_sent(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                if event.player.uuid() == 12 {
                    self.0.lock().unwrap().push((
                        event.player.permission_level(host),
                        event.tree.root_names().any(|name| name == "deop"),
                    ));
                }
                None
            }
        }

        let reads = Reads::default();
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "observe_deop",
                Box::new(Observe(reads.clone())),
            )]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    game.options.execution_role = ExecutionRole::DedicatedServer;
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.ui.dispatcher.0 =
                        Arc::new(sunrise_command::vanilla::build_for(false, true));
                    let files = tempfile::tempdir().unwrap();
                    let root = files.path();
                    let mut ops = OpList::new(root.join("ops.json"));
                    assert!(ops.add(OpEntry {
                        user: NameAndId {
                            id: 12,
                            name: "guest12".into()
                        },
                        level: 4,
                        bypasses_player_limit: false,
                    }));
                    game.server.server_lists.0 = Some(Arc::new(Mutex::new(Lists {
                        ops,
                        white: WhiteList::new(root.join("whitelist.json")),
                        bans: BanList::new(root.join("banned-players.json")),
                        ip_bans: IpBanList::new(root.join("banned-ips.json")),
                        use_whitelist: false,
                        enforce_whitelist: false,
                        op_level: 4,
                        player_idle_timeout: 0,
                        stop: false,
                        autosave: true,
                    })));
                    let (sent, received) = std::sync::mpsc::channel();
                    let hub = &mut game.server.lan.as_mut().unwrap().hub;
                    hub.config.integrated = false;
                    hub.ops.insert(12, 4);
                    hub.refresh_permissions();
                    hub.update_guest(12, |g| g.sender = PlayerSender::new(sent, Arc::default()));
                    game.send_player_command_tree(12);
                    // GameProfileArgument requires a player-only selector.
                    let (ok, feedback) = game
                        .run_guest_command_told(GuestCommand {
                            conn: 12,
                            command: "deop @s[type=minecraft:player]".into(),
                            permission: 4,
                            integrated: false,
                            quiet: false,
                        })
                        .expect("the command source is online");
                    assert!(ok, "{feedback:?}");
                    assert_eq!(*reads.lock().unwrap(), [(Some(4), true), (Some(0), false)]);
                    let delivered: Vec<_> = command_delivery_packets(&received).collect();
                    let [
                        C::Commands(before),
                        C::EntityEvent(permission),
                        C::Commands(after),
                    ] = delivered.as_slice()
                    else {
                        panic!(
                            "expected the operator tree, permission update and non-operator tree"
                        );
                    };
                    assert_eq!((permission.id, permission.event), (12, 24));
                    for (packet, operator) in [(before, true), (after, false)] {
                        let roots = &packet.nodes[packet.root as usize].children;
                        assert_eq!(
                            roots
                                .iter()
                                .any(|&i| packet.nodes[i as usize].name == "deop"),
                            operator
                        );
                        assert!(roots.iter().any(|&i| packet.nodes[i as usize].name == "me"));
                    }
                })
            },
        );
    }

    fn command_delivery_packets(
        receiver: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
    ) -> impl Iterator<Item = C> + '_ {
        use sunrise_protocol::{buf::Reader, ids::play::clientbound as ids};
        use sunrise_sim::lan::Out;
        receiver
            .try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect during command delivery"),
            })
            .filter_map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                match id {
                    ids::ENTITY_EVENT | ids::COMMANDS | ids::PLAYER_INFO_UPDATE => {
                        Some(C::decode(id, reader.rest()).unwrap())
                    }
                    _ => None,
                }
            })
    }

    #[test]
    #[ignore = "daily"]
    fn outgoing_command_filter_reads_each_live_recipient_and_isolates_panics() {
        use serde_json::json;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_sim::lan::PlayerSender;

        struct Filter(Arc<Mutex<Vec<u128>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Filter {
            fn player_commands_sent(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                let uuid = event.player.uuid();
                assert_eq!(
                    event.player.name(host),
                    Some(if uuid == 12 { "guest12" } else { "guest13" })
                );
                if uuid == 12 {
                    assert_eq!(event.player.address(host), Some("127.0.0.1"));
                    assert_eq!(event.player.client_brand(host), Some("test-join-client"));
                    assert_eq!(
                        event.player.announced_channels(host).unwrap().as_ref(),
                        ["test:join"]
                    );
                }
                self.0.lock().unwrap().push(uuid);
                event
                    .tree
                    .hide_where(|name| uuid == 12 && name == "test_hide")
            }
        }

        struct Broken(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Broken {
            fn player_commands_sent(
                &self,
                _: &dyn mod_api::Host,
                _: &mut mod_api::PlayerCommandsSentEvent<'_>,
            ) -> Option<mod_api::CommandTreeSelection> {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("test command filter panicked");
            }
        }

        let calls = Arc::new(Mutex::new(Vec::new()));
        let panics = Arc::new(AtomicUsize::new(0));
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("broken_command_filter", Box::new(Broken(panics.clone()))),
                mod_api::ModEntry::new("command_filter", Box::new(Filter(calls.clone()))),
            ]
            .into(),
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    if game.native_lifecycle.epoch().is_none() {
                        game.native_lifecycle.world_open_ready().unwrap();
                    }
                    let host = game.server.lan.as_mut().unwrap();
                    let receivers = [12, 13].map(|conn| {
                        let (tx, rx) = std::sync::mpsc::channel();
                        host.hub.update_guest(conn, |g| {
                            g.sender = PlayerSender::new(tx, Arc::default());
                            g.placed = conn != 12;
                            if conn == 12 {
                                g.brand = "test-join-client".into();
                                assert!(
                                    g.sender.announced.apply("minecraft:register", b"test:join")
                                );
                            }
                        });
                        rx
                    });
                    assert!(host.hub.set_mod_commands(
                        0,
                        json!({"type": "root", "children": {
                            "test_hide": {"type": "literal", "executable": true},
                            "test_keep": {"type": "literal", "executable": true}
                        }})
                    ));
                    let unchanged = host.hub.commands_packet(0);
                    for (conn, rx) in [12, 13].into_iter().zip(receivers) {
                        if conn == 12 {
                            game.lan_guest_joined(conn, &[], [3.0, 64.0, 5.0], None);
                            assert!(
                                game.server
                                    .lan
                                    .as_ref()
                                    .unwrap()
                                    .hub
                                    .guest(conn)
                                    .unwrap()
                                    .placed
                            );
                        } else {
                            game.send_player_command_tree(conn);
                        }
                        let mut packets = command_delivery_packets(&rx);
                        let packet = packets
                            .find_map(|packet| match packet {
                                C::Commands(tree) => Some(tree),
                                C::PlayerInfoUpdate(info)
                                    if conn == 12
                                        && info.entries.iter().any(|row| row.uuid == 12) =>
                                {
                                    panic!(
                                        "the joining player was announced before its command tree"
                                    );
                                }
                                _ => None,
                            })
                            .expect("expected command tree");
                        if conn == 12 {
                            assert!(packets.any(|packet| matches!(packet,
                                C::PlayerInfoUpdate(info) if info.entries.iter().any(|row| row.uuid == 12)
                            )), "the joining player must be announced after its command tree");
                        }
                        if conn == 13 {
                            assert_eq!(
                                packet, unchanged,
                                "abstaining leaves the whole wire tree unchanged"
                            );
                        }
                        let names: Vec<_> = packet.nodes[packet.root as usize]
                            .children
                            .iter()
                            .map(|&id| packet.nodes[id as usize].name.as_str())
                            .filter(|name| name.starts_with("test_"))
                            .collect();
                        assert_eq!(
                            names,
                            if conn == 12 {
                                vec!["test_keep"]
                            } else {
                                vec!["test_hide", "test_keep"]
                            }
                        );
                    }
                    assert_eq!(*calls.lock().unwrap(), [12, 13]);
                    assert_eq!(panics.load(Ordering::Relaxed), 1);
                })
            },
        );
    }

    #[test]
    fn client_command_tree_refuses_server_only_handle_kinds_and_binds_slots() {
        use std::collections::HashMap;
        use sunrise_gameplay_common::game::native_server::{
            argument_kinds::command_arguments, client_command_tree, register_command_nodes,
        };
        let arguments = command_arguments();
        let server_only = [
            mod_api::Command::new("c_player <target:player>"),
            mod_api::Command::new("c_players <targets:players>"),
            mod_api::Command::new("c_entity <target:entity>"),
            mod_api::Command::new("c_entities <targets:entities>"),
            mod_api::Command::new("c_dim <dim:dimension>"),
        ];
        let (refused_tree, _, refused_problems) =
            client_command_tree(&server_only, &arguments, None);
        assert_eq!(
            refused_problems.len(),
            server_only.len(),
            "{refused_problems:?}"
        );
        for root in ["c_player", "c_players", "c_entity", "c_entities", "c_dim"] {
            assert!(
                refused_tree["children"].get(root).is_none(),
                "{root} should be refused on the client"
            );
        }

        let client_safe = [
            mod_api::Command::new("c_word <w:word>"),
            mod_api::Command::new("c_text <t:text>"),
            mod_api::Command::new("c_int <n:integer>"),
            mod_api::Command::new("c_num <x:number>"),
            mod_api::Command::new("c_bool <b:boolean>"),
            mod_api::Command::new("c_item <i:item>"),
            mod_api::Command::new("c_bpos <p:block_pos>"),
            mod_api::Command::new("c_vec <v:vec3>"),
            mod_api::Command::new("c_vecu <v:vec3_uncentered>"),
            mod_api::Command::new("c_vec2 <v:vec2>"),
            mod_api::Command::new("c_vec2u <v:vec2_uncentered>"),
            mod_api::Command::new("c_col <c:column_pos>"),
            mod_api::Command::new("c_rot <r:rotation>"),
            mod_api::Command::new("c_nbt <n:nbt>"),
            mod_api::Command::new("c_id <id:id>"),
            mod_api::Command::new("c_mod <m:mod_id>"),
            mod_api::Command::new("c_res <r:resource:minecraft:item>"),
        ];
        let mut slots = HashMap::new();
        let (tree, _, problems) = client_command_tree(&client_safe, &arguments, Some(&mut slots));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(slots.len(), client_safe.len());

        let mut dispatcher = sunrise_command::Dispatcher::new();
        register_command_nodes(
            &mut dispatcher,
            &tree,
            sunrise_command::MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        let reg = sunrise_command::Registries::default();
        let parsed = dispatcher.parse("c_int 42", &reg);
        let (ctx, _) = parsed.executable().unwrap();
        let key = ctx
            .executor
            .expect("client command node binds an executor key");
        assert_eq!(key.slot(), 2);
        assert!(dispatcher.owns_executor(key));

        // Rebuilding the client dispatcher invalidates keys from the previous generation.
        let mut rebuilt = sunrise_command::Dispatcher::new();
        register_command_nodes(
            &mut rebuilt,
            &tree,
            sunrise_command::MOD_COMMAND,
            &arguments,
            Some(&slots),
        );
        assert!(!rebuilt.owns_executor(key));
    }

    #[test]
    #[ignore = "daily"]
    fn command_refresh_rebuilds_the_typed_dispatcher_only_on_request() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_server_server::tests::command_refresh_rebuilds_the_typed_dispatcher_only_on_request",
            || {
                use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
                struct ChangingCommands {
                    changed: Arc<AtomicBool>,
                    registrations: Arc<AtomicUsize>,
                    calls: Arc<Mutex<Vec<Option<&'static str>>>>,
                    replacement: bool,
                }
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for ChangingCommands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let changed = self.changed.load(Ordering::Relaxed);
                        if !self.replacement {
                            self.registrations.fetch_add(1, Ordering::Relaxed);
                            let usage = if changed {
                                "after <n:integer>"
                            } else {
                                "before"
                            };
                            event
                                .commands
                                .push(mod_api::Command::new(usage).executes(|_, _, _| Ok(1)));
                        }
                        if changed == self.replacement {
                            let calls = self.calls.clone();
                            let result = if self.replacement { 37 } else { 0 };
                            event
                                .commands
                                .push(mod_api::Command::new("retained").executes(
                                    move |_, _, _| {
                                        calls
                                            .lock()
                                            .unwrap()
                                            .push(mod_api::content::crash::current_native_mod());
                                        Ok(result)
                                    },
                                ));
                        }
                    }
                    fn player_commands_sent(
                        &self,
                        _: &dyn mod_api::Host,
                        event: &mut mod_api::PlayerCommandsSentEvent<'_>,
                    ) -> Option<mod_api::CommandTreeSelection> {
                        event.tree.hide_where(|name| name == "before")
                    }
                }
                let changed = Arc::new(AtomicBool::new(false));
                let registrations = Arc::new(AtomicUsize::new(0));
                let calls = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    [("refresh", false), ("replacement", true)]
                        .map(|(id, replacement)| {
                            mod_api::ModEntry::new(
                                id,
                                Box::new(ChangingCommands {
                                    changed: changed.clone(),
                                    registrations: registrations.clone(),
                                    calls: calls.clone(),
                                    replacement,
                                }),
                            )
                        })
                        .into(),
                    || {
                        super::super::native_guest_turn_tests::with_guests(|game| {
                            game.build_registries();
                            let (sent, received) = std::sync::mpsc::channel();
                            game.server.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                                g.permission = 2;
                                g.sender =
                                    sunrise_sim::lan::PlayerSender::new(sent, Arc::default());
                            });
                            game.register_native_commands();
                            let delivered: Vec<_> = command_delivery_packets(&received).collect();
                            let [C::Commands(tree)] = delivered.as_slice() else {
                                panic!("installing a changed display report sends only its tree");
                            };
                            let roots = &tree.nodes[tree.root as usize].children;
                            assert!(
                                roots
                                    .iter()
                                    .any(|&i| tree.nodes[i as usize].name == "retained")
                            );
                            assert!(
                                !roots
                                    .iter()
                                    .any(|&i| tree.nodes[i as usize].name == "before")
                            );
                            assert_eq!(
                                game.server
                                    .lan
                                    .as_ref()
                                    .unwrap()
                                    .hub
                                    .guest(12)
                                    .unwrap()
                                    .permission,
                                2
                            );
                            assert_eq!(registrations.load(Ordering::Relaxed), 1);
                            let old_parse =
                                game.ui.dispatcher.0.parse("retained", &game.ui.registries);
                            let old_context = old_parse.executable().unwrap().0;
                            let (result, outcome) = game.recording_command_outcome(|game| {
                                game.execute_line("retained", game.server_source())
                            });
                            assert_eq!(result, Ok(0));
                            assert_eq!(outcome, Some((true, 0)));
                            assert_eq!(*calls.lock().unwrap(), [Some("refresh")]);
                            changed.store(true, Ordering::Relaxed);
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 1);
                            let registered =
                                game.server.native_commands.0.registered.as_ref().unwrap();
                            assert!(registered.levels.contains_key("before"));
                            mod_api::Commands::refresh_commands(game);
                            mod_api::Commands::refresh_commands(game);
                            assert_eq!(
                                registrations.load(Ordering::Relaxed),
                                1,
                                "requests are deferred"
                            );
                            game.register_native_commands();
                            assert!(!game.server.native_commands.0.refresh);
                            assert_eq!(
                                registrations.load(Ordering::Relaxed),
                                2,
                                "requests coalesce"
                            );
                            assert_eq!(
                                game.run_native_command(old_context.clone())
                                    .unwrap_err()
                                    .key,
                                "command.unknown.command"
                            );
                            assert_eq!(*calls.lock().unwrap(), [Some("refresh")]);
                            assert_eq!(game.execute_line("retained", game.server_source()), Ok(37));
                            assert_eq!(
                                *calls.lock().unwrap(),
                                [Some("refresh"), Some("replacement")]
                            );
                            assert!(mod_api::__private::set_mod_enabled("replacement", false));
                            assert_eq!(
                                game.execute_line("retained", game.server_source())
                                    .unwrap_err()
                                    .key,
                                "command.unknown.command"
                            );
                            assert_eq!(
                                *calls.lock().unwrap(),
                                [Some("refresh"), Some("replacement")]
                            );
                            assert!(mod_api::__private::set_mod_enabled("replacement", true));
                            let commands =
                                game.server.native_commands.0.registered.as_ref().unwrap();
                            assert!(!commands.levels.contains_key("before"));
                            assert!(commands.levels.contains_key("after"));
                            let dispatcher = &game.ui.dispatcher.0;
                            assert!(
                                dispatcher
                                    .parse("after 7", &game.ui.registries)
                                    .executable()
                                    .is_ok()
                            );
                            assert!(
                                dispatcher
                                    .parse("after nope", &game.ui.registries)
                                    .executable()
                                    .is_err()
                            );
                            assert!(
                                dispatcher
                                    .parse("before", &game.ui.registries)
                                    .executable()
                                    .is_err()
                            );
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 2);
                            mod_api::Commands::refresh_commands(game);
                            game.register_native_commands();
                            assert_eq!(registrations.load(Ordering::Relaxed), 3);
                            assert!(game.server.native_commands.0.level("after").is_some());
                            changed.store(false, Ordering::Relaxed);
                            game.reload_datapacks(false, &game.level.enabled_features.clone())
                                .unwrap();
                            let (sent, received) = std::sync::mpsc::channel();
                            game.server
                                .lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(12, |guest| {
                                    guest.sender = sunrise_sim::lan::PlayerSender::new(
                                        sent,
                                        Default::default(),
                                    );
                                })
                                .unwrap();
                            game.register_native_commands();
                            let packet = received
                                .try_iter()
                                .flat_map(|out| match out {
                                    sunrise_sim::lan::Out::Packet(body) => vec![body],
                                    sunrise_sim::lan::Out::Packets(bodies) => bodies,
                                    sunrise_sim::lan::Out::Disconnect(_) => {
                                        panic!("reload disconnected guest")
                                    }
                                })
                                .find_map(|body| {
                                    let mut reader = sunrise_protocol::buf::Reader::new(&body);
                                    let id = reader.varint().unwrap();
                                    match C::decode(id, &body[reader.position()..]).unwrap() {
                                        C::Commands(packet) => Some(packet),
                                        _ => None,
                                    }
                                })
                                .expect("reload must deliver a command tree");
                            let names: Vec<_> = packet.nodes[packet.root as usize]
                                .children
                                .iter()
                                .map(|&i| packet.nodes[i as usize].name.as_str())
                                .collect();
                            assert!(!names.is_empty());
                            assert!(
                                !names.contains(&"before"),
                                "new reload root must be filtered"
                            );
                            assert!(!names.contains(&"after"), "old root must be retired");
                        })
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_command_feedback_respects_track_output_and_silence() {
        use sunrise_gameplay::game::server_side::commands::execute::Source;
        use sunrise_level::chunk::block_entity::BlockEntity;
        with_command_sources(|game| {
            let source = Source {
                block: Some((1, 64, 1)),
                silent: false,
                ..game.server_source()
            };
            for track in [true, false] {
                if let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get_mut(&(1, 64, 1))
                {
                    block.track_output = track;
                    block.last_output = None;
                }
                game.with_source(source.clone(), |game| {
                    game.native_feedback(Text::literal("answer"), false)
                });
                let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get(&(1, 64, 1))
                else {
                    panic!("block missing")
                };
                assert_eq!(block.last_output.is_some(), track);
            }
            game.server.chat_capture = Some(Vec::new());
            game.with_source(
                Source {
                    silent: true,
                    block: None,
                    ..source
                },
                |game| game.native_feedback(Text::literal("silent"), false),
            );
            assert!(game.server.chat_capture.take().unwrap().is_empty());
            assert!(game.server.command_source.is_none());
            game.as_guest(12, |game| {
                let source = game.server_source();
                game.with_source(source, |game| {
                    assert!(matches!(
                        game.native_command_origin(4),
                        Ok(mod_api::CommandSource::Console { .. })
                    ));
                });
            })
            .unwrap();
            game.server.exec.0.functions.push("test:outer".into());
            assert!(game.native_run_command("list"));
            assert_eq!(game.server.exec.0.functions, ["test:outer"]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn native_commands_keep_function_block_and_cart_sources() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_server_server::tests::native_commands_keep_function_block_and_cart_sources",
            || {
                use mod_api::{CommandSource, PermissionLevel as Permission};
                use sunrise_core::position::BlockPos;
                use sunrise_level::{
                    chunk::block_entity::BlockEntity,
                    datapack::{Function, FunctionLine},
                };
                struct Commands(Arc<Mutex<Vec<CommandSource>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let heard = self.0.clone();
                        let execute =
                            move |host: &mut dyn mod_api::ServerHost,
                                  source: &CommandSource,
                                  _: &mod_api::CommandInvocation| {
                                heard.lock().unwrap().push(source.clone());
                                host.commands()
                                    .feedback(mod_api::Text::literal("source answer"), false)
                                    .unwrap();
                                Ok(1)
                            };
                        event.commands.extend([
                            mod_api::Command::new("origin").executes(execute.clone()),
                            mod_api::Command::op("denied", 4).executes(execute),
                            // Replaced by the later accepted registration; failure must not fall back here.
                            mod_api::Command::new("unhandled").executes(|_, _, _| Ok(99)),
                            mod_api::Command::new("unhandled")
                                .executes(|_, _, _| Err(mod_api::Text::literal("refused").into())),
                        ]);
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "origins",
                        Box::new(Commands(heard.clone())),
                    )]
                    .into(),
                    || {
                        with_command_sources(|game| {
                            game.command_block_tick(BlockPos::new(1, 64, 1));
                            let Some(BlockEntity::CommandBlock(block)) =
                                game.world.block_entities.get(&(1, 64, 1))
                            else {
                                panic!("block missing")
                            };
                            assert_eq!(block.success_count, 1);
                            let source = heard.lock().unwrap()[0].clone();
                            assert!(
                                matches!(source, CommandSource::CommandBlock { position: [1, 64, 1], level, .. } if level == game.dim.key())
                            );
                            assert_eq!(source.permission(), Permission::Gamemasters);
                            assert!(game.perform_cart_command(99));
                            assert_eq!(game.carts[0].command.success_count, 1);
                            assert!(game.carts[0].command.last_output.is_some());
                            assert!(
                                matches!(heard.lock().unwrap().last().unwrap(), CommandSource::CommandBlockMinecart { entity, .. } if entity.id() == 99)
                            );
                            game.server.chat_capture = Some(Vec::new());
                            let mut function = Function {
                                id: "test:inner".into(),
                                lines: vec![
                                    FunctionLine::Command(
                                        "execute positioned 2 64 3 run origin".into(),
                                    ),
                                    FunctionLine::Command("denied".into()),
                                ],
                                parameters: vec![],
                            };
                            game.ensure_datapacks();
                            game.server
                                .fns
                                .0
                                .store
                                .functions
                                .insert(function.id.clone(), function.clone());
                            function.id = "test:outer".into();
                            function.lines =
                                vec![FunctionLine::Command("function test:inner".into())];
                            let before = heard.lock().unwrap().len();
                            game.run_function(&function, None, game.server_source())
                                .unwrap();
                            assert_eq!(heard.lock().unwrap().len(), before + 1);
                            assert!(
                                matches!(heard.lock().unwrap().last().unwrap(), CommandSource::Function { id, permission } if id.to_string() == "test:inner" && *permission == Permission::Gamemasters)
                            );
                            assert!(
                                game.server.chat_capture.take().unwrap().is_empty(),
                                "functions suppress both success and failure"
                            );
                            assert!(game.server.exec.0.functions.is_empty());
                            game.ui.registries.permission = 4;
                            assert!(!game.run_command("unhandled"));
                            assert!(game.run_command(
                                "execute store result storage test:result value int 1 run origin"
                            ));
                            assert!(game.run_command(
                                "execute if data storage test:result {value:1} run origin"
                            ));
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_typed_commands_reach_listeners_after_parse_and_source_resolution() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::native_server_server::tests::native_typed_commands_reach_listeners_after_parse_and_source_resolution",
            || {
                use mod_api::{CommandArgumentValue as Value, CommandInvocation};
                struct Commands(Arc<Mutex<Vec<(bool, CommandInvocation)>>>);
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        let heard = self.0.clone();
                        let record =
                            move |_: &mut dyn mod_api::ServerHost,
                                  source: &mod_api::CommandSource,
                                  command: &CommandInvocation| {
                                if command.literals == ["typed", "give"] {
                                    let Some(Value::Integer(amount)) = command.argument("amount")
                                    else {
                                        panic!("executor must receive an integer")
                                    };
                                    let Some(Value::Player(player)) = command.argument("who")
                                    else {
                                        panic!("executor must receive a resolved player")
                                    };
                                    assert_ne!(*amount, 0);
                                    assert_ne!(player.uuid(), 0);
                                }
                                let console =
                                    matches!(source, mod_api::CommandSource::Console { .. });
                                heard.lock().unwrap().push((console, command.clone()));
                                Ok(1)
                            };
                        event.commands.extend([
                            mod_api::Command::new("typed give <who:player> <amount:integer> [message:text]"),
                            mod_api::Command::new("typed data <word:word> <number:number> <enabled:boolean> <item:item> <pos:block_pos>"),
                            mod_api::Command::new("typed opt [value:integer]"),
                            mod_api::Command::new("/set <x:word>"),
                            mod_api::Command::new("typed repeat <value:player> <value:integer>"),
                            mod_api::Command::new("typed at <pos:vec3>"),
                            mod_api::Command::op("protected <who:player>", 4),
                        ].map(|command| command.executes(record.clone())));
                    }
                }
                let heard = Arc::new(Mutex::new(Vec::new()));
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "typed_commands",
                        Box::new(Commands(heard.clone())),
                    )]
                    .into(),
                    || {
                        super::super::native_guest_turn_tests::with_guests(|game| {
                            game.build_registries();
                            game.register_native_commands();
                            let other = game.player(13).unwrap();
                            let acting = game.player(12).unwrap();
                            game.as_guest(12, |game| {
                                for line in [
                                    "typed give guest13 nope",
                                    "typed give guest13",
                                    "typed give missing 7",
                                    "typed give @a 7",
                                    "typed data word 1 maybe stone ~ ~ ~",
                                    "typed data word NaN true stone ~ ~ ~",
                                    "typed data word 1 true does_not_exist ~ ~ ~",
                                    "typed opt 1 extra",
                                ] {
                                    assert_eq!(game.perform_native_command(12, 4, line, false), Some(false), "{line}");
                                }
                                let random = game.world.random.state();
                                assert_eq!(game.perform_native_command(12, 0, "protected @r", false), None);
                                assert_eq!(game.world.random.state(), random, "denied selectors must not resolve");
                                assert!(heard.lock().unwrap().is_empty(), "failed input must not reach listeners");
                                for line in [
                                    "typed give guest13 37 hello typed world",
                                    "typed give @s -4",
                                    "typed opt",
                                    "typed opt 9",
                                    "typed data word -1.25 true stone[minecraft:custom_name='\"Rock\"'] ~1 ~2 ~-3",
                                    "typed data word 2 false dirt ^0 ^0 ^0",
                                ] {
                                    assert_eq!(game.perform_native_command(12, 4, line, false), Some(true), "{line}");
                                }
                            }).unwrap();
                            game.spawn = Some(sunrise_core::geometry::Vec3::new(10.5, 70.0, -2.5));
                            game.yaw = 45.0;
                            game.pitch = 30.0;
                            for (line, key) in [
                                ("typed give guest13 nope", "parsing.int.expected"),
                                ("typed give missing 1", "argument.entity.notfound.player"),
                            ] {
                                game.server.chat_capture = Some(Vec::new());
                                assert!(game.perform_native_console_command(line));
                                let output = game.server.chat_capture.take().unwrap();
                                assert_eq!(
                                    output[0].plain(),
                                    Text::translate(key, &[]).plain(),
                                    "{line}"
                                );
                            }
                            assert!(game.perform_native_console_command("typed give @s 1"));
                            assert_eq!(
                                heard.lock().unwrap().len(),
                                6,
                                "console has no source player"
                            );
                            assert!(game.perform_native_console_command("typed give guest13 11"));
                            assert!(game.perform_native_console_command(
                                "typed data console 3 true dirt ~1 ~2 ~3"
                            ));
                            assert!(game.perform_native_console_command(
                                "typed data console 3 true dirt ^0.5 ^0.5 ^0.5"
                            ));
                            assert!(game.perform_native_console_command("typed repeat guest13 3"));
                            assert!(game.perform_native_console_command("typed at 1 64.25 ~1"));
                            // A point is three numbers: two, or text, is a parse error and is not heard
                            // (the count of 12 heard lines below holds neither).
                            game.perform_native_console_command("typed at 1 64.25");
                            game.perform_native_console_command("typed at 1 up 3");
                            assert!(
                                game.server.command_source.is_none(),
                                "console source restored"
                            );
                            game.as_guest(12, |game| {
                                assert!(
                                    game.native_run_command("execute as guest13 run typed opt 13")
                                )
                            })
                            .unwrap();
                            let observations = heard.clone();
                            let heard = heard.lock().unwrap();
                            assert_eq!(heard.len(), 12);
                            let first = &heard[0].1;
                            assert_eq!(first.literals, ["typed", "give"]);
                            assert_eq!(first.argument("who"), Some(&Value::Player(other)));
                            assert_eq!(first.argument("amount"), Some(&Value::Integer(37)));
                            assert_eq!(
                                first.argument("message"),
                                Some(&Value::String("hello typed world".into()))
                            );
                            assert_eq!(heard[1].1.argument("who"), Some(&Value::Player(acting)));
                            assert_eq!(heard[1].1.argument("amount"), Some(&Value::Integer(-4)));
                            assert!(heard[1].1.argument("message").is_none());
                            assert!(heard[2].1.arguments.is_empty());
                            assert_eq!(heard[3].1.argument("value"), Some(&Value::Integer(9)));
                            let data = &heard[4].1;
                            assert_eq!(data.argument("word"), Some(&Value::String("word".into())));
                            assert_eq!(data.argument("number"), Some(&Value::Number(-1.25)));
                            assert_eq!(data.argument("enabled"), Some(&Value::Boolean(true)));
                            assert_eq!(data.argument("pos"), Some(&Value::BlockPos([4, 66, 2])));
                            let Some(Value::Item(item)) = data.argument("item") else {
                                panic!("typed item")
                            };
                            assert_eq!(
                                mod_api::__private::item_key_id(item.item()),
                                Some("minecraft:stone")
                            );
                            assert_eq!(item.count(), 1);
                            assert!(item.components().is_some(), "command components retained");
                            assert_eq!(
                                heard[5].1.argument("enabled"),
                                Some(&Value::Boolean(false))
                            );
                            assert_eq!(
                                heard[5].1.argument("pos"),
                                Some(&Value::BlockPos([3, 64, 5]))
                            );
                            assert!(heard[..6].iter().all(|(console, _)| !console));
                            assert!(heard[6..].iter().all(|(console, _)| *console));
                            assert_eq!(heard[6].1.argument("who"), Some(&Value::Player(other)));
                            assert_eq!(heard[6].1.argument("amount"), Some(&Value::Integer(11)));
                            assert_eq!(
                                heard[7].1.argument("pos"),
                                Some(&Value::BlockPos([11, 72, 0]))
                            );
                            assert_eq!(
                                heard[8].1.argument("pos"),
                                Some(&Value::BlockPos([10, 70, -3]))
                            );
                            assert_eq!(heard[9].1.arguments[0].value, Value::Player(other));
                            assert_eq!(heard[9].1.argument("value"), Some(&Value::Integer(3)));
                            // `1 64.25 ~1`: a whole x is a block centre (1.5), y is as typed, and `~1`
                            // is one block from the console's z.
                            assert_eq!(
                                heard[10].1.argument("pos"),
                                Some(&Value::Vec3([1.5, 64.25, -2.0]))
                            );
                            drop(heard);
                            game.server
                                .lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(12, |g| g.permission = 4)
                                .unwrap();
                            let inventory = game
                                .server
                                .lan
                                .as_ref()
                                .unwrap()
                                .hub
                                .guest(12)
                                .unwrap()
                                .inventory
                                .clone();
                            game.guest_command(12, "/set stone", None);
                            game.guest_command(12, "typed opt 17", None);
                            game.guest_command(12, "/give guest12 minecraft:stone 1", None);
                            game.run_guest_commands();
                            let heard = observations.lock().unwrap();
                            assert_eq!(heard.len(), 14);
                            assert_eq!(heard[12].1.literals, ["/set"]);
                            assert_eq!(
                                heard[12].1.argument("x"),
                                Some(&Value::String("stone".into()))
                            );
                            assert_eq!(heard[13].1.literals, ["typed", "opt"]);
                            assert_eq!(heard[13].1.argument("value"), Some(&Value::Integer(17)));
                            assert_eq!(
                                game.server
                                    .lan
                                    .as_ref()
                                    .unwrap()
                                    .hub
                                    .guest(12)
                                    .unwrap()
                                    .inventory,
                                inventory
                            );
                        });
                    },
                );
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod fake_action_tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;

    use sunrise_gameplay_common::game::native_server::fake_action_tests::on_top;

    use sunrise_gameplay::game::native_server::fake_action_tests::fake;

    use sunrise_gameplay::game::native_server::fake_action_tests::held;

    use sunrise_gameplay::game::native_server::fake_action_tests::place;

    use sunrise_gameplay::game::respawn_anchor::RespawnConfig;

    use sunrise_gameplay_common::game::stations_common::{Kind, Station};

    use sunrise_gameplay_common::game::native_server::Acting;

    use mod_api::{ActionOutcome, ActionRefused, FakeEffect, PlayerAction};

    use mod_api::{BlockHit, Hand};

    use sunrise_items::items::ItemStack;

    #[test]
    #[ignore = "daily"]
    fn a_fakes_action_is_refused_before_anything_runs() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:water_bucket"), 0);
            // Out of reach: nothing used, the bucket still full.
            let far = PlayerAction::UseOn {
                hand: Hand::Main,
                pos: [40, 63, 0],
                hit: BlockHit::new([40.5, 64.0, 0.5], [0, 1, 0], false),
            };
            assert_eq!(
                game.player_perform_action(player, far),
                Err(ActionRefused::OutOfReach)
            );
            assert_eq!(
                held(game, 2).map(|s| s.item.to_string()).as_deref(),
                Some("minecraft:water_bucket")
            );
            // A real player's handle.
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action(real, on_top([1, 63, 0])),
                Err(ActionRefused::NotFake)
            );
            // Inside another action, as a hook running during it would ask.
            let acting = Acting::start(&game.server.fake_action.0);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert!(!game.server.fake_action.0.acting());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_fakes_action_leaves_the_per_action_state_as_it_was() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            let before = sunrise_gameplay_common::game::Hit {
                pos: [7, 7, 7],
                face: [1, 0, 0],
                point: [7.0, 7.5, 7.5],
                inside: true,
            };
            game.server.forced_hit = Some(before);
            assert_eq!(
                game.player_perform_action(player, on_top([1, 63, 0])),
                Ok(ActionOutcome::Consumed)
            );
            let after = game.server.forced_hit.expect("the forced hit is put back");
            assert_eq!(
                (after.pos, after.face, after.point, after.inside),
                (before.pos, before.face, before.point, before.inside)
            );
            assert!(game.server.guest_use.is_none());
        });
    }
    /// A fake player's click on a shulker box or an ender chest is refused
    /// at the open: no open count climbs, no stand-in chest enters the level
    /// and no lid opens.
    #[test]
    #[ignore = "daily"]
    fn a_shulker_box_and_an_ender_chest_a_fake_player_used_are_left_shut() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let shulker = [1, 64, 0];
            place(game, shulker, "minecraft:shulker_box");
            let count = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server
                    .be_anims
                    .shulkers
                    .get(&shulker)
                    .map(|s| s.open_count)
            };
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the use is refused at the open, nothing opened"
            );
            assert!(game.container.is_none());
            assert_eq!(
                game.player_perform_action(player, on_top(shulker)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu)),
                "the next use is refused the same way",
            );
            assert!(
                count(game).is_none_or(|c| c == 0),
                "the count doesn't climb"
            );

            let diamonds = Some(ItemStack::new("minecraft:diamond", 5));
            game.server.ender_items[0] = diamonds.clone();
            let ender = [2, 64, 0];
            place(game, ender, "minecraft:ender_chest");
            let key = (ender[0], ender[1], ender[2]);
            // A raw block state has no stand-in. Establish the closed state
            // whose preservation this refused action is meant to check.
            game.open_ender_chest(key);
            game.close_ender_chest();
            game.container = None;
            assert_eq!(
                game.player_perform_action(player, on_top(ender)),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert!(game.server.ender_open.is_none());
            match game.world.block_entities.get(&key) {
                Some(sunrise_level::chunk::block_entity::BlockEntity::Chest(items)) => assert!(
                    items.iter().all(Option::is_none),
                    "the player's items stayed"
                ),
                other => panic!("the ender chest lost its block entity: {other:?}"),
            }
            assert_eq!(
                game.server.ender_items[0], diamonds,
                "the server's own ender items changed"
            );
            assert!(
                !game.server.lid_openers.contains_key(&key),
                "its lid stayed open"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_hooks_see_the_setup_and_fail_is_not_consumed() {
        use mod_api::{Flow, Hooks, HostSide, PlayerActionSetup, RightClickBlockEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        #[derive(Default)]
        struct Deny(Arc<AtomicBool>);
        #[mod_api::sunrise_mod]
        impl Hooks for Deny {
            fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent) -> Flow {
                let HostSide::Server(host) = host else {
                    panic!("server action");
                };
                assert_eq!(event.click.player, 2);
                let entity = host.entity(event.click.player_entity_id as u32).unwrap();
                assert_eq!(entity.rotation(host), Some([123.0, 17.0]));
                assert_eq!(
                    entity.eye_position(host),
                    Some([0.5, 64.0 + f64::from(1.62_f32), 0.5])
                );
                assert_eq!(entity.is_shift_key_down(host), Some(true));
                self.0.store(true, Ordering::Relaxed);
                *event.cancellation_result = mod_api::InteractionResult::Fail;
                Flow::Cancel
            }
        }
        let heard = Arc::new(AtomicBool::new(false));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "deny",
                Box::new(Deny(heard.clone())),
            )]
            .into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let (player, ..) = fake(game, Some("minecraft:diamond"), 0);
                    game.server.use_failed = true;
                    let result = player
                        .perform_action_scoped(
                            game,
                            on_top([1, 63, 0]),
                            PlayerActionSetup::new(
                                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                                    &ItemStack::new("minecraft:oak_log", 1),
                                )),
                                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                                123.0,
                                17.0,
                                true,
                            ),
                        )
                        .unwrap();
                    assert!(
                        heard.load(Ordering::Relaxed),
                        "normal Host-bearing hook ran"
                    );
                    assert_eq!(result.outcome, ActionOutcome::Passed);
                    assert_eq!(
                        result.held,
                        Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                            &ItemStack::new("minecraft:oak_log", 1)
                        ))
                    );
                    assert!(game.server.use_failed, "outer per-action flag restored");
                    let level = game.level(game.dim.id()).unwrap();
                    assert_eq!(
                        game.level_block(level, [1, 64, 0])
                            .and_then(mod_api::__private::block_key_id),
                        Some("minecraft:air")
                    );
                    assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn scoped_fake_use_uses_viewpoint_secondary_use_and_restores_on_refusal() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, conn, _) = fake(game, Some("minecraft:diamond"), 1);
            let setup = |item, sneaking| {
                PlayerActionSetup::new(
                    Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                        &ItemStack::new(item, 1),
                    )),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    90.0,
                    sneaking,
                )
            };
            // Air use rays from the supplied eyes/look, not the persistent fake's view.
            let mut viewpoint = setup("minecraft:water_bucket", false);
            viewpoint.eye = [2.5, 64.0 + f64::from(1.62_f32), 2.5];
            viewpoint.yaw = 117.0;
            let result = game
                .player_perform_action_scoped(
                    player,
                    PlayerAction::UseItem { hand: Hand::Main },
                    viewpoint,
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held,
                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:bucket", 1)
                ))
            );
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                game.level_block(level, [2, 64, 2])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:water")
            );
            place(game, [1, 64, 0], "minecraft:chest");
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", false),
                )
                .unwrap();
            assert_eq!(
                result.outcome,
                ActionOutcome::EffectBlocked(FakeEffect::Menu)
            );
            let result = game
                .player_perform_action_scoped(
                    player,
                    on_top([1, 64, 0]),
                    setup("minecraft:oak_log", true),
                )
                .unwrap();
            assert_eq!(result.outcome, ActionOutcome::Consumed);
            assert_eq!(
                result.held, None,
                "survival placement consumes the temporary stack"
            );
            assert_eq!(
                game.level_block(level, [1, 65, 0])
                    .and_then(mod_api::__private::block_key_id),
                Some("minecraft:oak_log")
            );
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([40, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::OutOfReach)
            );
            let mut invalid = setup("minecraft:oak_log", false);
            invalid.eye[0] = f64::NAN;
            assert_eq!(
                game.player_perform_action_scoped(player, on_top([1, 63, 0]), invalid),
                Err(ActionRefused::InvalidSetup)
            );
            let real = game.player(1).unwrap();
            assert_eq!(
                game.player_perform_action_scoped(
                    real,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::NotFake)
            );
            let acting = Acting::start(&game.server.fake_action.0);
            assert_eq!(
                game.player_perform_action_scoped(
                    player,
                    on_top([1, 63, 0]),
                    setup("minecraft:oak_log", false)
                ),
                Err(ActionRefused::Busy)
            );
            drop(acting);
            assert_eq!(held(game, 2), Some(ItemStack::new("minecraft:diamond", 1)));
            let guest = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.game_mode, 1);
            assert_eq!(guest.position, [0.5, 64.0, 0.5]);
        });
    }
    /// Each field holds a value the use can't have written, so a swap that
    /// forgets one fails whether the use changed it or not.
    #[test]
    #[ignore = "daily"]
    fn the_servers_own_player_state_comes_back_after_a_fake_use() {
        use mod_api::PlayerActionSetup;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (player, ..) = fake(game, None, 0);
            let dim = game.dim;
            let config = RespawnConfig {
                dim,
                pos: [1, 2, 3],
                yaw: 0.0,
                pitch: 0.0,
                forced: false,
            };
            let spawn = sunrise_core::geometry::Vec3::new(1.0, 2.0, 3.0);
            game.station = Some(Station::new_kind(Kind::Anvil, [4, 5, 6]));
            game.server.craft_table = Some([7, 8, 9]);
            game.experience.level = 7;
            game.server.respawn_config = Some(config);
            game.spawn = Some(spawn);
            // An ender inventory left open goes back while the fake player
            // acts, and its stand-in returns afterwards, as a guest's turn does.
            let stand_in = (12, 64, 8);
            game.open_ender_chest(stand_in);
            game.container = None;

            place(game, [1, 64, 0], "minecraft:crafting_table");
            assert_eq!(
                game.player_perform_action(player, on_top([1, 64, 0])),
                Ok(ActionOutcome::EffectBlocked(FakeEffect::Menu))
            );
            assert_eq!(
                game.station.as_ref().map(|s| (s.kind, s.pos)),
                Some((Kind::Anvil, [4, 5, 6])),
            );
            assert_eq!(game.server.craft_table, Some([7, 8, 9]));
            assert_eq!(game.experience.level, 7);
            assert_eq!(game.server.respawn_config, Some(config));
            assert_eq!(game.spawn, Some(spawn));
            assert_eq!(game.server.ender_open, Some(stand_in));
            assert!(game.world.block_entities.get(&stand_in).is_some());

            let setup = PlayerActionSetup::new(
                Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:fishing_rod", 1),
                )),
                [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                0.0,
                0.0,
                false,
            );
            let rod = player
                .perform_action_scoped(game, PlayerAction::UseItem { hand: Hand::Main }, setup)
                .unwrap();
            assert_eq!(rod.outcome, ActionOutcome::Consumed, "the rod casts");
            assert!(game.hook.is_none(), "no hook is left for nobody to reel in");
        });
    }

    /// A level-tick listener runs inside its own dispatch transaction, yet can act through a fake
    /// player directly; one the listener opens itself still refuses. Pre uses the scoped action
    /// Create's deployer uses; post the plain one.
    #[test]
    #[ignore = "daily"]
    fn a_level_tick_listener_acts_through_its_fake_player() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{
            Hooks, HostSide, HostTransaction, LevelTickPostEvent, LevelTickPreEvent, ModEntry,
            PlayerActionSetup, PlayerRef,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay_common::game::{
            native_hooks::base::GameHooks as _, transactions::TransactionsGameExt as _,
        };

        type Answer = Result<ActionOutcome, ActionRefused>;
        #[derive(Default)]
        struct Seen {
            player: Option<PlayerRef>,
            setup: Option<PlayerActionSetup>,
            answers: Vec<(&'static str, bool, Answer, Answer)>,
        }
        struct Deployer(Arc<Mutex<Seen>>);

        #[mod_api::__subscriptions]
        impl Hooks for Deployer {
            fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
                let HostSide::Server(host) = host else { return };
                let mut seen = self.0.lock().unwrap();
                let (Some(player), Some(setup)) = (seen.player, seen.setup.clone()) else {
                    return;
                };
                let boundary = !host.transaction_matches(None);
                let direct = player
                    .perform_action_scoped(host, on_top([0, 63, 1]), setup.clone())
                    .map(|result| result.outcome);
                let nested = HostTransaction::transaction(host, |host, _| {
                    Ok::<_, ()>(
                        player
                            .perform_action_scoped(host, on_top([0, 63, -1]), setup)
                            .map(|result| result.outcome),
                    )
                })
                .unwrap();
                seen.answers.push(("pre", boundary, direct, nested));
            }
            fn level_tick_post(&self, host: HostSide, _: &mut LevelTickPostEvent) {
                let HostSide::Server(host) = host else { return };
                let mut seen = self.0.lock().unwrap();
                let Some(player) = seen.player else { return };
                let boundary = !host.transaction_matches(None);
                let direct = player.perform_action(host, on_top([1, 63, 0]));
                let nested = HostTransaction::transaction(host, |host, _| {
                    Ok::<_, ()>(player.perform_action(host, on_top([-1, 63, 0])))
                })
                .unwrap();
                seen.answers.push(("post", boundary, direct, nested));
            }
        }

        let seen = Arc::new(Mutex::new(Seen::default()));
        let mods = vec![ModEntry::new("deployer", Box::new(Deployer(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
                seen.lock().unwrap().player = Some(player);
                seen.lock().unwrap().setup = Some(PlayerActionSetup::new(
                    Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                        &ItemStack::new("minecraft:oak_log", 1),
                    )),
                    [0.5, 64.0 + f64::from(1.62_f32), 0.5],
                    0.0,
                    60.0,
                    false,
                ));
                let level = game.level(game.dim.id()).unwrap();
                game.native_level_tick_pre(level);
                game.native_level_tick_post(level);
                let used = |stage| {
                    (
                        stage,
                        true,
                        Ok(ActionOutcome::Consumed),
                        Err(ActionRefused::Busy),
                    )
                };
                assert_eq!(seen.lock().unwrap().answers, [used("pre"), used("post")]);
                let log = game.level_block_property(level, [0, 64, 1], "axis");
                assert_eq!(
                    log,
                    Some("y"),
                    "the pre listener's scoped use placed its log"
                );
                let log = game.level_block_property(level, [1, 64, 0], "axis");
                assert_eq!(log, Some("y"), "the post listener's use placed the log");
                assert_eq!(held(game, 2), None);
                assert!(!game.has_open_transaction());
            });
        });
    }

    /// A mod's `block_interaction_range` (#5824) decides a block's reach: a use 10 blocks away is
    /// out of reach for vanilla, and goes through once a mod answers for that block.
    #[test]
    #[ignore = "daily"]
    fn a_mods_interaction_range_lets_a_player_act_on_a_far_block() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::{BlockInteractionRangeEvent, Hooks, ModEntry};

        const FAR: [i32; 3] = [10, 63, 0];
        struct RemoteReach;
        #[mod_api::__subscriptions]
        impl Hooks for RemoteReach {
            fn block_interaction_range(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut BlockInteractionRangeEvent,
            ) {
                let [x, y, z] = FAR;
                if event.pos == mod_api::BlockPos::new(x, y, z) {
                    event.within = true;
                }
            }
        }

        // The client library reaches the hook through the game's installed callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            let (player, ..) = fake(game, Some("minecraft:oak_log"), 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(FAR[0], FAR[1], FAR[2], stone).unwrap();
            let far = |game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]| {
                let host = game.server.lan.as_ref().unwrap();
                let g = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap();
                g.within_interaction_range(&game.loop_context, pos, 4.0)
            };
            assert_eq!(
                game.player_perform_action(player, on_top(FAR)),
                Err(ActionRefused::OutOfReach)
            );
            assert!(!far(game, FAR));
            let mods = vec![ModEntry::new("remote_reach", Box::new(RemoteReach))].into();
            mod_platform::testing::with_mods(mods, || {
                assert!(far(game, FAR));
                assert!(
                    !far(game, [10, 63, 3]),
                    "only the block the mod answers for"
                );
                assert_eq!(
                    game.player_perform_action(player, on_top(FAR)),
                    Ok(ActionOutcome::Consumed)
                );
            });
            let level = game.level(game.dim.id()).unwrap();
            let log = game.level_block_property(level, [10, 64, 0], "axis");
            assert_eq!(log, Some("y"), "the far use placed the log");
        });
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
