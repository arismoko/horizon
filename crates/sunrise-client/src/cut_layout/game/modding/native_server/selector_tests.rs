//! Acceptance tests through the registered native command dispatcher and real server selectors.

use mod_api::{CommandArgumentValue as Value, CommandInvocation};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_server::{
    ArgType, Dispatcher, HashMap, command_tree, json, register_dispatcher,
};
use sunrise_sim::mobs::Mob;

pub(crate) struct SelectorCommands(pub(crate) Arc<Mutex<Vec<CommandInvocation>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for SelectorCommands {
    fn register_commands(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        event.commands.extend(
            [
                "heal <targets:players>",
                "selectone <target:entity>",
                "selectall <targets:entities>",
                "selectplayer <target:player>",
                "optionalplayers [targets:players]",
                "optionalentity [target:entity]",
                "optionalentities [targets:entities]",
            ]
            .map(|usage| {
                let heard = self.0.clone();
                mod_api::Command::new(usage).executes(move |_, _, command| {
                    heard.lock().unwrap().push(command.clone());
                    Ok(1)
                })
            }),
        );
    }
}

#[test]
fn selector_kinds_publish_vanilla_flags_and_share_parser_suggestions() {
    let commands = [
        mod_api::Command::new("one <target:entity>"),
        mod_api::Command::new("many <target:entities>"),
        mod_api::Command::new("person <target:player>"),
        mod_api::Command::new("people <target:players>"),
    ];
    let (tree, _, problems) = command_tree(
        &commands,
        &sunrise_command::Dispatcher::new(),
        &HashMap::new(),
    );
    assert!(problems.is_empty(), "{problems:?}");
    let mut dispatcher = Dispatcher::new();
    register_dispatcher(
        &mut dispatcher,
        &tree,
        "native_mod_command",
        &HashMap::new(),
    );
    let mut registries = sunrise_command::Registries {
        permission: 4,
        players: vec!["Alice".into(), "Bob".into()],
        selected_entities: vec!["00000000-0000-0000-0000-000000000007".into()],
        ..Default::default()
    };
    for (name, amount, kind, single, players) in [
        ("one", "single", "entities", true, false),
        ("many", "multiple", "entities", false, false),
        ("person", "single", "players", true, true),
        ("people", "multiple", "players", false, true),
    ] {
        let node = &tree["children"][name]["children"]["target"];
        assert_eq!(node["parser"], "minecraft:entity");
        assert_eq!(node["properties"], json!({"amount": amount, "type": kind}));
        let root = dispatcher.nodes[0]
            .children
            .iter()
            .copied()
            .find(|id| {
                matches!(&dispatcher.nodes[*id].kind,
                sunrise_command::NodeKind::Literal(literal) if literal == name)
            })
            .unwrap();
        let argument = dispatcher.nodes[root].children[0];
        assert!(matches!(&dispatcher.nodes[argument].kind,
            sunrise_command::NodeKind::Argument {
                ty: ArgType::Entity { single: s, players: p }, ..
            } if *s == single && *p == players));
        for (prefix, expected) in [("Al", "Alice"), ("@", "@a")] {
            let input = format!("{name} {prefix}");
            let parsed = dispatcher.parse(&input, &registries);
            let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
            assert!(
                suggestions
                    .iter()
                    .any(|suggestion| suggestion.text == expected),
                "{input}: {suggestions:?}"
            );
        }
        let input = format!("{name} 000");
        let parsed = dispatcher.parse(&input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert_eq!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.text == "00000000-0000-0000-0000-000000000007"),
            !players
        );
        registries.permission = 0;
        let input = format!("{name} @");
        let parsed = dispatcher.parse(&input, &registries);
        let (_, suggestions) = dispatcher.suggestions(&parsed, input.len(), &registries);
        assert!(
            !suggestions
                .iter()
                .any(|suggestion| suggestion.text.starts_with('@'))
        );
        registries.permission = 4;
    }
}

#[test]
#[ignore = "daily"]
fn entity_selection_has_no_additional_count_cap() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_server::selector_tests::entity_selection_has_no_additional_count_cap",
        || {
            let heard = Arc::new(Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "selector_uncapped",
                    Box::new(SelectorCommands(heard.clone())),
                )]
                .into(),
                || {
                    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                        game.build_registries();
                        game.register_native_commands();
                        let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                        // Reversed insertion order rules out accidental list/ID ordering.
                        for id in (1000..1300).rev() {
                            let x = id - 1000 + 4;
                            sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                                &mut game.world,
                                ((x >> 4) as i32, 0),
                            );
                            game.mobs.push(Mob::new(
                                id,
                                cow.clone(),
                                Vec3::new(f64::from(x), 64.0, 5.0),
                                0.0,
                            ));
                        }
                        let epoch = game.player(12).unwrap().entity().epoch();
                        game.as_guest(12, |game| {
                            assert_eq!(
                                game.perform_native_command(
                                    12,
                                    4,
                                    "selectall @e[type=minecraft:cow,sort=nearest]",
                                    false,
                                ),
                                Some(true)
                            );
                        })
                        .unwrap();
                        let calls = heard.lock().unwrap();
                        assert_eq!(calls.len(), 1);
                        assert_eq!(
                            calls[0].argument("targets"),
                            Some(&Value::Entities(
                                (1000..1300)
                                    .map(|id| mod_api::EntityRef::new(epoch, id))
                                    .collect()
                            ))
                        );
                    });
                },
            );
        },
    );
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::native_server_server::selector_tests::in_two_levels;

/// `native_command_entity_target` looks a selected entity up in the level that holds it
/// (`in_target_level`): a mod command run from the overworld reaches the entity that lives in
/// the nether, and each tag picks its own cow, not the one with the other id.
#[test]
#[ignore = "daily"]
fn a_typed_entity_argument_reaches_an_entity_in_another_level() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_server::selector_tests::a_typed_entity_argument_reaches_an_entity_in_another_level",
        || {
            in_two_levels(&[
                (
                    "selectone @e[type=minecraft:cow,tag=over,limit=1]",
                    "target",
                    &[1],
                ),
                (
                    "selectone @e[type=minecraft:cow,tag=under,limit=1]",
                    "target",
                    &[2],
                ),
            ]);
        },
    );
}

/// A selector with no positional option reaches every level; `distance` (like `x`, `y`, `z`,
/// `dx`, `dy` and `dz`) confines it to the source's level, which `execute in` sets; and
/// `execute as` makes the chosen entity `@s`, not the invoker.
#[test]
#[ignore = "daily"]
fn execute_moves_the_selector_source_to_its_level_and_entity() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_server::selector_tests::execute_moves_the_selector_source_to_its_level_and_entity",
        || {
            in_two_levels(&[
                // No positional option: both levels, whatever level the source is in.
                ("selectall @e[type=minecraft:cow]", "targets", &[1, 2]),
                (
                    "execute in minecraft:the_nether run selectall @e[type=minecraft:cow]",
                    "targets",
                    &[1, 2],
                ),
                (
                    "execute in minecraft:the_nether run selectall @e[type=minecraft:cow,tag=over]",
                    "targets",
                    &[1],
                ),
                // Confined by `distance`: the level of the source `execute in` made.
                (
                    "execute in minecraft:the_nether run selectall @e[type=minecraft:cow,distance=..1000]",
                    "targets",
                    &[2],
                ),
                (
                    "execute in minecraft:overworld run selectall @e[type=minecraft:cow,distance=..1000]",
                    "targets",
                    &[1],
                ),
                (
                    "selectall @e[type=minecraft:cow,distance=..1000]",
                    "targets",
                    &[1],
                ),
                // `as` the overworld cow: `@s` is that cow, not the invoker.
                (
                    "execute as @e[type=minecraft:cow,tag=over] run selectone @s",
                    "target",
                    &[1],
                ),
            ]);
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
