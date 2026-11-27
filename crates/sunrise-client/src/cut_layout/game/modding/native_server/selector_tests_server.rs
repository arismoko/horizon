use crate::game::native_server::selector_tests::SelectorCommands;
use mod_api::CommandArgumentValue as Value;
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::{Dim, test_fixtures::fixture_resident_column};
use sunrise_sim::mobs::Mob;

/// One overworld cow (cow 1, tag `over`) and one nether cow (cow 2, tag `under`), then `lines`
/// run as the first guest with the console's permission; each line's last invocation names the
/// expected cows in its `argument` (their numbers, sorted). The cows take the next entity ids in
/// summon order, after those the guests reserved.
pub(crate) fn in_two_levels(lines: &[(&str, &str, &[u32])]) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "selector_two_levels",
            Box::new(SelectorCommands(heard.clone())),
        )]
        .into(),
        || {
            super::super::native_guest_turn_tests::with_resident_guests(|game| {
                game.enter_level(Dim::Nether).unwrap();
                fixture_resident_column(&mut game.world, (0, 0));
                game.enter_level(Dim::Overworld).unwrap();
                game.build_registries();
                game.register_native_commands();
                game.ui.registries.permission = 4;
                let before_cows = game.server.next_entity;
                assert!(game.run_command(r#"summon minecraft:cow 4 64 5 {Tags:["over"]}"#));
                assert!(game.run_command(
                    r#"execute in minecraft:the_nether run summon minecraft:cow 4 64 5 {Tags:["under"]}"#
                ));
                game.lan_assign_ids();
                game.as_guest(12, |game| {
                    for (line, argument, expected) in lines {
                        game.ui.registries.permission = 4;
                        let before = heard.lock().unwrap().len();
                        assert!(game.run_command(line), "{line}");
                        let calls = heard.lock().unwrap();
                        assert_eq!(calls.len(), before + 1, "{line} reaches the mod once");
                        let mut ids = match calls.last().unwrap().argument(argument) {
                            Some(Value::Entity(entity)) => vec![entity.id()],
                            Some(Value::Entities(entities)) => {
                                entities.iter().map(|entity| entity.id()).collect()
                            }
                            other => panic!("{line}: {other:?}"),
                        };
                        ids.sort_unstable();
                        let expected: Vec<u32> =
                            expected.iter().map(|cow| before_cows + cow).collect();
                        assert_eq!(ids, expected, "{line}");
                    }
                })
                .unwrap();
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn native_selectors_deliver_ordered_handles_once_and_refuse_invalid_selections() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_server_server::selector_tests::native_selectors_deliver_ordered_handles_once_and_refuse_invalid_selections",
        || {
            let heard = Arc::new(Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "selector_acceptance",
                    Box::new(SelectorCommands(heard.clone())),
                )]
                .into(),
                || {
                    super::super::native_guest_turn_tests::with_resident_guests(|game| {
                        game.build_registries();
                        game.register_native_commands();
                        let first = game.player(12).unwrap();
                        let second = game.player(13).unwrap();
                        let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                        // Deliberately neither ID order nor insertion order is distance order.
                        for (id, x) in [(101, 12.0), (103, 4.0), (102, 7.0)] {
                            game.mobs
                                .push(Mob::new(id, cow.clone(), Vec3::new(x, 64.0, 5.0), 0.0));
                        }
                        let near = game.entity(103).unwrap();
                        let middle = game.entity(102).unwrap();
                        let far = game.entity(101).unwrap();
                        game.as_guest(12, |game| {
                            assert_eq!(
                                game.perform_native_command(12, 4, "heal @a", false),
                                Some(true)
                            );
                            {
                                let calls = heard.lock().unwrap();
                                assert_eq!(
                                    calls.len(),
                                    1,
                                    "one invocation, not one per selected player"
                                );
                                assert_eq!(
                                    calls[0].argument("targets"),
                                    Some(&Value::Players(vec![first, second]))
                                );
                            }
                            for line in [
                                "selectone @e[type=minecraft:cow,limit=1,sort=nearest]",
                                "selectall @e[type=minecraft:cow,sort=furthest]",
                                "selectall @e[type=minecraft:cow,sort=nearest]",
                            ] {
                                assert_eq!(
                                    game.perform_native_command(12, 4, line, false),
                                    Some(true),
                                    "{line}"
                                );
                            }
                            {
                                let calls = heard.lock().unwrap();
                                assert_eq!(calls.len(), 4);
                                assert_eq!(calls[1].argument("target"), Some(&Value::Entity(near)));
                                assert_eq!(
                                    calls[2].argument("targets"),
                                    Some(&Value::Entities(vec![far, middle, near]))
                                );
                                assert_eq!(
                                    calls[3].argument("targets"),
                                    Some(&Value::Entities(vec![near, middle, far]))
                                );
                            }
                            // Selector permission is independent of the command's permission (all
                            // these commands are public). Names must still work at level zero.
                            let random = game.world.random.state();
                            for line in
                                ["heal @a", "selectone @r", "selectall @e", "selectplayer @s"]
                            {
                                game.server.chat_capture = Some(Vec::new());
                                assert_eq!(
                                    game.perform_native_command(12, 0, line, false),
                                    Some(false),
                                    "{line}"
                                );
                                let output = game.server.chat_capture.take().unwrap();
                                assert_eq!(
                                    output[0].plain(),
                                    Text::translate("argument.entity.selector.not_allowed", &[])
                                        .plain(),
                                    "{line}"
                                );
                            }
                            assert_eq!(game.world.random.state(), random);
                            assert_eq!(
                                heard.lock().unwrap().len(),
                                4,
                                "denied selectors never dispatch"
                            );
                            for line in [
                                "heal guest13",
                                "selectone guest13",
                                "selectall guest13",
                                "selectplayer guest13",
                            ] {
                                assert_eq!(
                                    game.perform_native_command(12, 0, line, false),
                                    Some(true),
                                    "{line}"
                                );
                            }
                            for line in [
                                "optionalplayers",
                                "optionalentity",
                                "optionalentities",
                                "optionalplayers @a",
                                "optionalentity @s",
                                "optionalentities @s",
                            ] {
                                assert_eq!(
                                    game.perform_native_command(12, 4, line, false),
                                    Some(true),
                                    "{line}"
                                );
                            }
                        })
                        .unwrap();
                        {
                            let calls = heard.lock().unwrap();
                            assert_eq!(calls.len(), 14);
                            assert_eq!(
                                calls[4].argument("targets"),
                                Some(&Value::Players(vec![second]))
                            );
                            assert_eq!(
                                calls[5].argument("target"),
                                Some(&Value::Entity(second.entity()))
                            );
                            assert_eq!(
                                calls[6].argument("targets"),
                                Some(&Value::Entities(vec![second.entity()]))
                            );
                            assert_eq!(calls[7].argument("target"), Some(&Value::Player(second)));
                            assert!(calls[8..11].iter().all(|call| call.arguments.is_empty()));
                            assert_eq!(
                                calls[11].argument("targets"),
                                Some(&Value::Players(vec![first, second]))
                            );
                            assert_eq!(
                                calls[12].argument("target"),
                                Some(&Value::Entity(first.entity()))
                            );
                            assert_eq!(
                                calls[13].argument("targets"),
                                Some(&Value::Entities(vec![first.entity()]))
                            );
                        }
                        for (line, key) in [
                            ("heal @a[name=missing]", "argument.entity.notfound.player"),
                            ("selectplayer missing", "argument.entity.notfound.player"),
                            (
                                "selectall @e[type=minecraft:cow,name=missing]",
                                "argument.entity.notfound.entity",
                            ),
                            (
                                "selectone @e[type=minecraft:cow,name=missing,limit=1]",
                                "argument.entity.notfound.entity",
                            ),
                            ("selectplayer @a", "argument.player.toomany"),
                            (
                                "selectone @e[type=minecraft:cow]",
                                "argument.entity.toomany",
                            ),
                            (
                                "optionalplayers @a[name=missing]",
                                "argument.entity.notfound.player",
                            ),
                            (
                                "optionalentities @e[type=minecraft:cow,name=missing]",
                                "argument.entity.notfound.entity",
                            ),
                        ] {
                            game.server.chat_capture = Some(Vec::new());
                            assert!(game.perform_native_console_command(line), "{line}");
                            let output = game.server.chat_capture.take().unwrap();
                            assert_eq!(
                                output[0].plain(),
                                Text::translate(key, &[]).plain(),
                                "{line}"
                            );
                            assert_eq!(
                                heard.lock().unwrap().len(),
                                14,
                                "{line} must not reach the listener"
                            );
                        }

                        // Resident mobs remain selector candidates while their native handle
                        // is temporarily held out during a mutation.
                        game.server.held_out.push(103);
                        assert!(game.entity(103).is_none());
                        for line in [
                            "selectall @e[type=minecraft:cow,x=3,y=64,z=5,distance=..2]",
                            "selectone @e[type=minecraft:cow,x=3,y=64,z=5,limit=1,sort=nearest]",
                        ] {
                            game.server.chat_capture = Some(Vec::new());
                            assert!(game.perform_native_console_command(line));
                            let output = game.server.chat_capture.take().unwrap();
                            assert_eq!(
                                output[0].plain(),
                                Text::translate("argument.entity.notfound.entity", &[]).plain()
                            );
                            assert_eq!(heard.lock().unwrap().len(), 14);
                        }
                        // Drop only the unavailable handle, preserving the remaining order.
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
                        game.server.held_out.pop();
                        let calls = heard.lock().unwrap();
                        assert_eq!(calls.len(), 15);
                        assert_eq!(
                            calls[14].argument("targets"),
                            Some(&Value::Entities(vec![middle, far]))
                        );
                    });
                },
            );
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
