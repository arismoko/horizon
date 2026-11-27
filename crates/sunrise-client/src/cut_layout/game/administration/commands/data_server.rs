#![cfg(test)]
use sunrise_core::nbt::Tag;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Compound;
    use sunrise_gameplay::game::commands::data::DataRef;
    use sunrise_gameplay::game::native_entity_refs::tests::{fixture_resident_column, with_game};
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

    /// `EntityDataAccessor.setData` keeps the entity's id and UUID: a
    /// `/data merge` that carries a different UUID does not take, and a
    /// display's load marks its start delay forced
    /// (`Display.setTransformationInterpolationDelay`).
    #[test]
    #[ignore = "daily"]
    fn merging_display_nbt_keeps_the_uuid_and_forces_the_start_delay() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let mut raw = Compound::new();
            raw.insert(
                "Pos".into(),
                Tag::List(
                    [0.5f64, 64.0, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            let raw = Tag::Compound(raw);
            let display = game
                .read_display_nbt("minecraft:block_display", &raw)
                .map(std::sync::Arc::new);
            game.persist.raw.insert(12, raw);
            game.persist.uuids.insert(12, [1, 2, 3, 4]);
            game.server.others.0.push(
                sunrise_gameplay_common::game::commands_common::OtherEntity {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    display,
                    knot_check: None,
                    eye: None,
                    interaction: None,
                    id: 12,
                    kind: "minecraft:block_display".into(),
                    position: Vec3::new(0.5, 64.0, 0.5),
                    yaw: 0.0,
                    pitch: 0.0,
                },
            );
            let mut merge = Compound::new();
            merge.insert(
                "Pos".into(),
                Tag::List(
                    [0.5f64, 64.0, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            merge.insert("UUID".into(), Tag::IntArray(vec![9, 9, 9, 9]));
            merge.insert("start_interpolation".into(), Tag::Int(0));
            merge.insert("view_range".into(), Tag::Float(2.5_f32.to_bits()));
            game.apply_entity_nbt(Target::Other(12), Tag::Compound(merge))
                .unwrap();
            assert_eq!(game.uuid_for(12), [1, 2, 3, 4]);
            assert_eq!(game.persist.forced_meta.get(&12), Some(&[1 << 8, 0, 0, 0]));
            let entity = game.entity(12).unwrap();
            assert_eq!(game.display(entity).unwrap().view_range(game), Some(2.5));
            assert!(game.persist.raw[&12].get("view_range").is_none());
            game.spend_forced_meta();
            let ctx = sunrise_command::Context {
                root: 0,
                nodes: vec![],
                args: vec![sunrise_command::ParsedArg {
                    name: "nbt".into(),
                    value: sunrise_command::Arg::Str("{start_interpolation:0}".into()),
                    range: (0, 23),
                }],
                command: None,
                executor: None,
                range: (0, 23),
                child: None,
                literals: vec![],
            };
            for _ in 0..2 {
                assert_eq!(
                    game.cmd_data_in("data.merge", &ctx, DataRef::Entity(Target::Other(12)))
                        .unwrap(),
                    1
                );
                assert_eq!(game.persist.forced_meta.get(&12), Some(&[1 << 8, 0, 0, 0]));
                game.spend_forced_meta();
            }
        });
    }
}

#[cfg(test)]
#[path = "data/eye_item_tests_server.rs"]
pub(crate) mod eye_item_tests;

#[cfg(test)]
#[path = "data/saved_tag_tests_server.rs"]
pub(crate) mod saved_tag_tests;

#[cfg(test)]
pub(crate) mod modern_literal_tests {
    use super::*;
    use sunrise_gameplay::game::commands::execute::ExecState;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn admitted_modern_literals_reach_actual_storage_as_typed_tags() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let run = |g: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            g.server.exec.0 = ExecState::default();
            let source = g.player_source();
            g.execute_line(line, source)
        };
        let cases = [
            ("hex", "0x10", "16", Tag::Int(16)),
            ("underscore", "1_000", "1000", Tag::Int(1000)),
            ("unicode4", r#""\u0041""#, r#""A""#, Tag::String("A".into())),
            ("hex2", r#""\x41""#, r#""A""#, Tag::String("A".into())),
            (
                "unicode8",
                r#""\U00000041""#,
                r#""A""#,
                Tag::String("A".into()),
            ),
            (
                "trailing",
                "{n:3,}",
                "{n:3}",
                Tag::compound([("n", Tag::Int(3))]),
            ),
            ("plain", "plain", r#""plain""#, Tag::String("plain".into())),
        ];
        let clock = game.world.game_time;
        let mut differences = Vec::new();
        for (name, modern, plain, expected) in &cases {
            for route in ["merge", "modify"] {
                let target = format!("h189:{route}_{name}");
                let reference = format!("h189:ref_{route}_{name}");
                let seed = ["data merge storage ", &target, r#" {v:"seed"}"#].concat();
                assert_eq!(run(&mut game, &seed).unwrap(), 1);
                assert_eq!(
                    game.score.storage.get(&target).get("v"),
                    Some(&Tag::String("seed".into()))
                );
                let canonical = ["data merge storage ", &reference, " {v:", plain, "}"].concat();
                assert_eq!(run(&mut game, &canonical).unwrap(), 1);
                assert_eq!(game.score.storage.get(&reference).get("v"), Some(expected));
                let command = if route == "merge" {
                    ["data merge storage ", &target, " {v:", modern, "}"].concat()
                } else {
                    ["data modify storage ", &target, " v set value ", modern].concat()
                };
                let dispatcher = sunrise_command::vanilla::game_dispatcher();
                assert!(
                    dispatcher
                        .parse(&command, &game.ui.registries)
                        .executable()
                        .is_ok(),
                    "admission: {route}/{name}"
                );
                let result = run(&mut game, &command);
                let actual = game.score.storage.get(&target).get("v").cloned();
                if result.as_ref().ok() != Some(&1) || actual.as_ref() != Some(expected) {
                    differences.push(format!("{route}/{name}: result={result:?}, stored={actual:?}, expected={expected:?}"));
                }
                // Copy actual canonical stored data back: equality is a real
                // no-change failure, independent of a formatter or parser stub.
                let copy = [
                    "data modify storage ",
                    &target,
                    " v set from storage ",
                    &reference,
                    " v",
                ]
                .concat();
                if run(&mut game, &copy).is_ok() {
                    differences.push(format!(
                        "{route}/{name}: canonical copy changed the stored Tag"
                    ));
                }
                assert_eq!(game.score.storage.get(&target).get("v"), Some(expected));
            }
        }
        assert_eq!(game.world.game_time, clock);
        assert!(differences.is_empty(), "{}", differences.join("\n"));
    }
}

#[cfg(test)]
pub(crate) mod falling_live_adoption_tests {
    use crate::game::server_side::commands::data::falling_live_adoption_tests::with_game;
    use sunrise_core::nbt::Tag;
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_gameplay_common::game::commands_common::Target;

    #[test]
    #[ignore = "daily"]
    fn commands_mutate_the_resident_falling_model_without_rejoining() {
        with_game(|game| {
            let uuid = "000003d4-0000-0000-0000-000000000001";
            assert!(game.run_command("summon falling_block 4.5 200 5.5 {UUID:[I;980,0,0,1],BlockState:\"minecraft:gravel\",Time:5,DropItem:1b,NoGravity:1b,Motion:[0d,0d,0d],Tags:[keep],Silent:1b}"));
            assert!(game.run_command(&format!(
                "execute as {uuid} if entity @s[type=falling_block]"
            )));
            let index = game
                .falling
                .iter()
                .position(|f| game.persist.uuid_of(f.id) == Some([980, 0, 0, 1]))
                .unwrap();
            game.falling.update(index, |falling| {
                falling.start_pos = BlockPos::new(9, 201, 11);
                falling.stuck = Vec3::new(0.2, 0.3, 0.4);
                falling.portal.cooldown = 17;
                falling.portal.process = Some(sunrise_sim::nether_portal::Process {
                    entry: BlockPos::new(8, 199, 8),
                    time: 7,
                    inside: true,
                });
            });
            let before = game.falling[index].clone();
            assert!(before.join.is_joined());
            assert_eq!((before.age, before.drop_item), (5, true));
            let next_id = game.server.next_entity;
            let clock = game.world.game_time;
            assert!(!game.run_command(&format!("data merge entity {uuid} {{Time:5}}")));
            assert!(
                game.run_command(&format!("data merge entity {uuid} {{Time:12}}")),
                "valid Time update must reach model"
            );
            assert_eq!(game.falling[index].age, 12);
            assert!(game.run_command(&format!("data merge entity {uuid} {{DropItem:0b}}")));
            assert!(!game.falling[index].drop_item);
            let f = &game.falling[index];
            assert_eq!(
                (f.id, f.join, f.start_pos, f.stuck),
                (before.id, before.join, before.start_pos, before.stuck)
            );
            assert_eq!(
                (
                    f.position,
                    f.previous,
                    f.velocity,
                    f.rotation,
                    f.state,
                    f.no_gravity,
                    f.damage,
                    f.fall_distance
                ),
                (
                    before.position,
                    before.position,
                    before.velocity,
                    before.rotation,
                    before.state,
                    before.no_gravity,
                    before.damage,
                    before.fall_distance
                )
            );
            assert_eq!(f.portal.cooldown, before.portal.cooldown);
            let (a, b) = (f.portal.process.unwrap(), before.portal.process.unwrap());
            assert_eq!((a.entry, a.time, a.inside), (b.entry, b.time, b.inside));
            assert_eq!(f.saved.as_ref().unwrap().0, [980, 0, 0, 1]);
            assert_eq!(game.persist.uuid_of(f.id), Some([980, 0, 0, 1]));
            let saved = game.command_target_nbt(Target::Falling(f.id)).unwrap();
            assert_eq!(saved.get("Time"), Some(&Tag::Int(12)));
            assert_eq!(saved.get("DropItem"), Some(&Tag::Byte(0)));
            assert_eq!(
                saved.get("BlockState"),
                Some(&Tag::String("minecraft:gravel".into()))
            );
            assert_eq!(
                saved.get("Tags"),
                Some(&Tag::List(vec![Tag::String("keep".into())]))
            );
            assert_eq!(saved.get("Silent"), Some(&Tag::Byte(1)));
            assert_eq!(game.falling.len(), 1);
            assert_eq!(game.server.next_entity, next_id);
            assert_eq!(game.world.game_time, clock);
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
