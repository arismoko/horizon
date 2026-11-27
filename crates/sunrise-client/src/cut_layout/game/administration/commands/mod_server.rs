#[cfg(test)]
#[path = "summon_item_orb_tests_server.rs"]
pub(crate) mod summon_item_orb_tests;

#[cfg(test)]
#[path = "summon_bolt_tests_server.rs"]
pub(crate) mod summon_bolt_tests;

#[cfg(test)]
pub(crate) mod peaceful_tests {
    use crate::game::server_side::commands::peaceful_tests::with_types;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_gameplay_common::game::server_side::commands::peaceful_tests::TRADER;

    #[test]
    #[ignore = "daily"]
    fn a_persisted_mod_type_despawns_when_difficulty_switches_to_peaceful() {
        with_types(ExecutionRole::IntegratedServer, |game| {
            // Restoring a saved entity still works in Peaceful. Its own
            // despawn check, not the save loader, enforces the type's rule.
            let saved = Tag::compound([
                ("id", Tag::String(TRADER.into())),
                (
                    "Pos",
                    Tag::List(
                        [4.5_f64, 64.0, 4.5]
                            .map(|v| Tag::Double(v.to_bits()))
                            .to_vec(),
                    ),
                ),
                ("PersistenceRequired", Tag::Byte(1)),
                ("NoAI", Tag::Byte(1)),
            ]);
            let id = game.load_mob(saved, None).unwrap();
            let trader = game.mobs.iter().find(|mob| mob.id == id).unwrap();
            assert!(trader.persistent && trader.no_ai);
            game.server
                .tickets
                .0
                .update(Some(sunrise_sim::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));

            game.set_difficulty_forced(2);
            game.tick_creature_entities().unwrap();
            assert!(game.mobs.iter().any(|mob| mob.id == id));
            game.set_difficulty_forced(0);
            game.tick_creature_entities().unwrap();
            assert!(!game.mobs.iter().any(|mob| mob.id == id));
        });
    }
}

#[cfg(test)]
#[path = "teleport_player_rider_tests_server.rs"]
pub(crate) mod teleport_player_rider_tests;

#[cfg(test)]
pub(crate) mod query_feedback_tests {

    #[test]
    #[ignore = "daily"]
    fn a_silent_source_drops_query_results_but_still_stores_them() {
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;

        let _publication = sunrise_sim::datapack_net::isolate_server_publication();
        with_game(|game| {
            game.server.world_root = game.options.game_dir.clone().unwrap();
            game.open_world_datapacks(&[], None).unwrap();
            game.set_allow_commands(true);
            assert!(game.run_command("time set 1234"));
            assert!(game.run_command("scoreboard objectives add q dummy"));
            let queries = [
                "time query time",
                "time query day",
                "time query day repetition",
                "time query gametime",
                "seed",
                "gamerule keep_inventory",
                "difficulty",
                "list",
                "help seed",
            ];
            // A function's source (silent, like `run_command_quietly`) is told nothing.
            for command in queries {
                game.server.chat_capture = Some(Vec::new());
                assert!(game.run_command_quietly(command), "{command}");
                let lines = game.server.chat_capture.take().unwrap();
                assert!(lines.is_empty(), "{command}: {:?}", lines[0].plain());
            }
            // The player's own source still sees each result.
            for command in queries {
                game.server.chat_capture = Some(Vec::new());
                assert!(game.run_command(command), "{command}");
                assert_eq!(
                    game.server.chat_capture.take().unwrap().len(),
                    1,
                    "{command}"
                );
            }
            // The result still reaches `execute store`.
            assert!(game.run_command_quietly("execute store result score #t q run time query day"));
            game.server.chat_capture = Some(Vec::new());
            assert!(game.run_command("scoreboard players get #t q"));
            let lines = game.server.chat_capture.take().unwrap();
            assert!(lines[0].plain().contains("1234"), "{:?}", lines[0].plain());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn gamerule_feedback_uses_short_ids_without_changing_registry_identity() {
        use std::sync::Arc;
        use sunrise_command::ParsedArg;
        use sunrise_game_data::gamerules::{GameRules, Rule, RuleRegistry, Value};
        use sunrise_gameplay::game::native_entity_refs::tests::with_game;
        use {sunrise_command::Arg, sunrise_command::Context};
        with_game(|game| {
            let registry = RuleRegistry::with_mod_rules(
                &["test:feedback".into()],
                &[Rule {
                    name: "test:spawn_monsters".into(),
                    category: "test:feedback".into(),
                    default: Value::Bool(true),
                    min: 0,
                    max: 0,
                }],
            )
            .unwrap();
            game.ui.rules = GameRules::with_registry(Arc::new(registry), &[]);
            for (input, display, stored) in [
                (
                    "spawn_monsters",
                    "spawn_monsters",
                    "minecraft:spawn_monsters",
                ),
                (
                    "minecraft:spawn_monsters",
                    "spawn_monsters",
                    "minecraft:spawn_monsters",
                ),
                (
                    "test:spawn_monsters",
                    "test:spawn_monsters",
                    "test:spawn_monsters",
                ),
            ] {
                game.ui.rules.set(stored, Value::Bool(true));
                let mut context = Context {
                    root: 0,
                    nodes: Vec::new(),
                    args: Vec::new(),
                    command: None,
                    executor: None,
                    range: (0, 0),
                    child: None,
                    literals: vec!["gamerule".into(), input.into()],
                };
                game.server.chat_capture = Some(Vec::new());
                assert_eq!(game.cmd_gamerule(&context).unwrap(), 1);
                let lines = game.server.chat_capture.take().unwrap();
                assert_eq!(lines.len(), 1);
                let component = lines[0].component();
                assert_eq!(component["translate"], "commands.gamerule.query");
                assert_eq!(component["with"][0]["text"], display);
                context.args.push(ParsedArg {
                    name: "value".into(),
                    value: Arg::Bool(false),
                    range: (0, 0),
                });
                game.server.chat_capture = Some(Vec::new());
                assert_eq!(game.cmd_gamerule(&context).unwrap(), 0);
                let lines = game.server.chat_capture.take().unwrap();
                assert_eq!(lines.len(), 1);
                let component = lines[0].component();
                assert_eq!(component["translate"], "commands.gamerule.set");
                assert_eq!(component["with"][0]["text"], display);
                let error = game.cmd_gamerule(&context).unwrap_err();
                assert_eq!(error.key, "commands.gamerule.not_set");
                assert_eq!(error.message.component()["with"][0]["text"], display);
                assert_eq!(game.ui.rules.get(stored), Some(Value::Bool(false)));
                assert_eq!(game.ui.rules.rule(input).unwrap().name, stored);
            }
        });
    }
}

#[cfg(test)]
#[path = "incoming_tests/knot_identity_server.rs"]
pub(crate) mod knot_identity_tests;

#[path = "world_cmds_server.rs"]
pub(crate) mod world_cmds;

#[path = "terrain_cmds_server.rs"]
pub(crate) mod terrain_cmds;

#[path = "misc_server.rs"]
pub(crate) mod misc;

#[path = "items_server.rs"]
pub(crate) mod items;

#[path = "execute_server.rs"]
pub(crate) mod execute;

#[path = "data_server.rs"]
pub(crate) mod data;

#[path = "compute_server.rs"]
pub(crate) mod compute;
