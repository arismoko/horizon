#![cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

#[cfg(test)]
pub(crate) mod passenger_relation_tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay::game::server_side::commands::execute::passenger_relation_tests::actor;
    use sunrise_gameplay::game::server_side::commands::execute::passenger_relation_tests::value;
    use sunrise_gameplay::game::server_side::commands::execute::passenger_relation_tests::vehicle;
    use sunrise_gameplay_common::game::commands_common::Target;

    #[test]
    #[ignore = "daily"]
    fn h150_actual_registered_passengers_select_direct_live_mob_and_boat_children() {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            for objective in ["h150u", "h150ur", "h150us"] {
                assert!(game.run_command(&format!("scoreboard objectives add {objective} dummy")));
            }
            let clock = game.world.game_time;
            // 1->2->3 is a nested mob tree; 4->5 a boat; 6->7,8 two direct children.
            let kinds = [
                "cow", "pig", "chicken", "oak_boat", "cow", "cow", "pig", "chicken", "pig", "cow",
            ];
            let mut ids = Vec::new();
            for (offset, kind) in kinds.into_iter().enumerate() {
                let i = offset + 1;
                let extra = if kind == "oak_boat" {
                    ""
                } else {
                    ",NoAI:1b,Health:1f"
                };
                assert!(game.run_command(&format!(
                "summon {kind} 8.5 200 8.5 {{UUID:[I;150,2,0,{i}],Tags:[\"h150u\"],NoGravity:1b,Silent:1b,Motion:[0d,0d,0d]{extra}}}"
            )));
                let words = [150, 2, 0, i as i32];
                let id = *game
                    .persist
                    .uuids
                    .iter()
                    .find(|(_, u)| **u == words)
                    .unwrap()
                    .0;
                let target = if kind == "oak_boat" {
                    Target::Boat(id)
                } else {
                    Target::Mob(id)
                };
                assert_eq!(game.target_type(target), format!("minecraft:{kind}"));
                assert_eq!(game.target_position(target), Some([8.5, 200.0, 8.5]));
                let saved = game.command_target_nbt(target).unwrap();
                assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(words.to_vec())));
                if kind != "oak_boat" {
                    assert_eq!(saved.get("Health"), Some(&Tag::Float(1f32.to_bits())));
                    assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
                }
                assert!(game.world.loaded(8, 8));
                assert!(game.run_command("execute if block 8 199 8 stone"));
                // Membership must come from runtime links; no retained Passengers body remains.
                game.persist.raw.remove(&id);
                ids.push(id);
            }
            for (child, parent) in [(2, 1), (3, 2), (5, 4), (7, 6), (8, 6)] {
                if child == 8 {
                    // Normal admission refuses the cow's second passenger; /ride forces it.
                    assert!(!game.start_riding_target(
                        Target::Mob(ids[child - 1]),
                        Target::Mob(ids[parent - 1]),
                        false,
                    ));
                }
                assert!(game.run_command(&format!(
                    "ride {} mount {}",
                    actor(child),
                    actor(parent)
                )));
                assert_eq!(
                    game.mobs
                        .iter()
                        .find(|m| m.id == ids[child - 1])
                        .unwrap()
                        .extra
                        .mount,
                    Some(ids[parent - 1])
                );
                vehicle(game, child, parent);
            }
            let mut failures = Vec::new();
            for (phase, parent, depth, selected) in [
                ("direct", 1, 1, vec![2]),
                ("nested", 1, 2, vec![3]),
                ("boat", 4, 1, vec![5]),
                ("multiple", 6, 1, vec![7, 8]),
                ("empty", 9, 1, vec![]),
            ] {
                for (objective, n) in [("h150u", 0), ("h150ur", 91), ("h150us", 17)] {
                    for i in 1..=10 {
                        assert!(game.run_command(&format!(
                            "scoreboard players set {} {objective} {n}",
                            actor(i)
                        )));
                    }
                }
                let nested = if depth == 2 { " on passengers" } else { "" };
                game.server.cmds.0.results = Some(Vec::new());
                game.server.chat_capture = Some(Vec::new());
                assert!(game.run_command(&format!(
                "execute as {} on passengers{nested} store result score @s h150ur store success score @s h150us run scoreboard players add @s h150u 1", actor(parent)
            )));
                let callbacks = game.server.cmds.0.results.take().unwrap();
                let messages: Vec<_> = game
                    .server
                    .chat_capture
                    .take()
                    .unwrap()
                    .iter()
                    .map(|t| t.plain())
                    .collect();
                println!("H150 actual {phase}: callbacks={callbacks:?}, feedback={messages:?}");
                let expected = vec![(true, 1); selected.len()];
                if selected.is_empty() {
                    assert_eq!(callbacks, expected, "independent empty parent callback");
                } else if callbacks != expected {
                    failures.push(format!("{phase} callbacks {callbacks:?} != {expected:?}"));
                }
                for i in 1..=10 {
                    let yes = selected.contains(&i);
                    let got = (
                        value(game, i, "h150u"),
                        value(game, i, "h150ur"),
                        value(game, i, "h150us"),
                    );
                    let want = if yes { (1, 1, 1) } else { (0, 91, 17) };
                    if phase == "empty" || !yes {
                        assert_eq!(
                            got, want,
                            "independent empty/unrelated/no-descendant control: {phase} {i}"
                        );
                    } else if got != want {
                        failures.push(format!("{phase} {i}: {got:?} != {want:?}"));
                    }
                }
            }
            assert!(game.run_command(&format!("ride {} dismount", actor(2))));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == ids[1])
                    .unwrap()
                    .extra
                    .mount,
                None
            );
            vehicle(game, 3, 2);
            for phase in ["dismounted", "remounted"] {
                if phase == "remounted" {
                    assert!(game.run_command(&format!("ride {} mount {}", actor(2), actor(1))));
                    vehicle(game, 2, 1);
                }
                for (objective, n) in [("h150u", 0), ("h150ur", 91), ("h150us", 17)] {
                    for i in 1..=10 {
                        assert!(game.run_command(&format!(
                            "scoreboard players set {} {objective} {n}",
                            actor(i)
                        )));
                    }
                }
                assert!(game.run_command(&format!(
                "execute as {} on passengers store result score @s h150ur store success score @s h150us run scoreboard players add @s h150u 1", actor(1)
            )));
                for i in [1, 2, 3, 10] {
                    let got = (
                        value(game, i, "h150u"),
                        value(game, i, "h150ur"),
                        value(game, i, "h150us"),
                    );
                    let want = if phase == "remounted" && i == 2 {
                        (1, 1, 1)
                    } else {
                        (0, 91, 17)
                    };
                    if phase == "dismounted" || i != 2 {
                        assert_eq!(
                            got, want,
                            "independent dismount/unrelated control: {phase} {i}"
                        );
                    } else if got != want {
                        failures.push(format!("{phase} {i}: {got:?} != {want:?}"));
                    }
                }
            }
            assert_eq!(game.world.game_time, clock, "zero actual observation ticks");
            assert!(
                failures.is_empty(),
                "actual registered direct passenger branch/callback/store failures: {failures:?}"
            );
        });
    }
}
