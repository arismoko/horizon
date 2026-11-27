#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
use sunrise_core::nbt::Tag;
#[cfg(test)]
use sunrise_core::position::ChunkPos;
#[cfg(test)]
use sunrise_level::chunk::Chunk;
#[cfg(test)]
use sunrise_sim::save::text;
#[cfg(test)]
#[path = "persist/anger_tests.rs"]
mod anger_tests;
#[cfg(test)]
#[path = "persist/lookup_order_tests.rs"]
pub(crate) mod lookup_order_tests;

#[cfg(test)]
#[path = "persist/tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
mod interaction_saved_defaults_tests {
    use super::*;
    use sunrise_sim::entity_nbt;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    #[test]
    #[ignore = "daily"]
    fn registered_interactions_save_default_and_loaded_settings() {
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
        options.game_dir = Some(std::env::current_dir().unwrap().join("game"));
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
        let mut chunk = Chunk::empty(0, 0);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        let time = game.world.game_time;
        assert!(game.run_command("summon marker 10.5 200 7.5 {UUID:[I;109,0,0,4],data:{audit:7}}"));
        assert!(game.run_command("summon pig 10.5 200 8.5 {UUID:[I;109,0,0,5],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Health:10f}"));
        let marker_id = game
            .server
            .others
            .0
            .iter()
            .find(|o| game.persist.uuids.get(&o.id) == Some(&[109, 0, 0, 4]))
            .unwrap()
            .id;
        let marker_before = game.command_target_nbt(Target::Other(marker_id)).unwrap();
        let mut ids = Vec::new();
        for (n, fields) in [
            (1, ""),
            (2, ",width:1f,height:1f,response:0b"),
            (
                3,
                ",width:2.5f,height:3.5f,response:1b,attack:{player:[I;109,1,0,91],timestamp:17L},interaction:{player:[I;109,1,0,92],timestamp:23L}",
            ),
        ] {
            assert!(game.run_command(&format!("summon interaction {} 200 4.5 {{UUID:[I;109,0,0,{n}],Rotation:[15f,-30f],CustomName:\"audit\",Tags:[\"audit\"]{fields}}}", 1.5+3.0*f64::from(n-1))));
            let o = game
                .server
                .others
                .0
                .iter()
                .find(|o| game.persist.uuids.get(&o.id) == Some(&[109, 0, 0, n]))
                .unwrap();
            assert_eq!(o.kind, "minecraft:interaction");
            ids.push(o.id);
        }
        assert_eq!(
            game.command_target_nbt(Target::Other(marker_id)).unwrap(),
            marker_before
        );
        for key in ["width", "height", "response"] {
            assert!(marker_before.get(key).is_none());
        }
        let pig_id = game
            .mobs
            .iter()
            .find(|m| game.persist.uuids.get(&m.id) == Some(&[109, 0, 0, 5]))
            .unwrap()
            .id;
        let pig = game.command_target_nbt(Target::Mob(pig_id)).unwrap();
        assert_eq!(pig.get("NoAI"), Some(&Tag::Byte(1)));
        assert_eq!(pig.get("PersistenceRequired"), Some(&Tag::Byte(1)));
        let saved: Vec<_> = game
            .entity_chunks(&[(0, 0)].into(), false)
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .collect();
        for (index, id) in ids.iter().enumerate() {
            let uuid = [109, 0, 0, index as i32 + 1];
            let command_tag = game.command_target_nbt(Target::Other(*id)).unwrap();
            let chunk_tag = saved
                .iter()
                .find(|tag| entity_nbt::uuid(tag) == Some(uuid))
                .unwrap();
            for tag in [&command_tag, chunk_tag] {
                assert_eq!(tag.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                assert_eq!(tag.get("CustomName"), Some(&Tag::String("audit".into())));
                assert_eq!(
                    tag.get("Tags"),
                    Some(&Tag::List(vec![Tag::String("audit".into())]))
                );
                assert_eq!(
                    tag.get("width"),
                    Some(&Tag::Float(
                        (if index == 2 { 2.5f32 } else { 1.0f32 }).to_bits()
                    )),
                    "width case {index}"
                );
                assert_eq!(
                    tag.get("height"),
                    Some(&Tag::Float(
                        (if index == 2 { 3.5f32 } else { 1.0f32 }).to_bits()
                    ))
                );
                assert_eq!(tag.get("response"), Some(&Tag::Byte(i8::from(index == 2))));
                if index == 2 {
                    for (key, n, time) in [("attack", 91, 17), ("interaction", 92, 23)] {
                        let record = tag.get(key).unwrap();
                        assert_eq!(
                            record.get("player"),
                            Some(&Tag::IntArray(vec![109, 1, 0, n]))
                        );
                        assert_eq!(record.get("timestamp"), Some(&Tag::Long(time)));
                    }
                } else {
                    assert!(tag.get("attack").is_none() && tag.get("interaction").is_none());
                }
            }
        }
        // The actual registered identity/position still supplies a fresh save without raw input.
        game.persist.raw.remove(&ids[0]);
        let fresh = game.command_target_nbt(Target::Other(ids[0])).unwrap();
        assert_eq!(fresh.get("UUID"), Some(&Tag::IntArray(vec![109, 0, 0, 1])));
        assert_eq!(fresh.get("width"), Some(&Tag::Float(1.0f32.to_bits())));
        assert_eq!(fresh.get("height"), Some(&Tag::Float(1.0f32.to_bits())));
        assert_eq!(fresh.get("response"), Some(&Tag::Byte(0)));
        assert_eq!(game.world.game_time, time);
    }
}

#[cfg(test)]
mod loaded_tag_tests {
    use super::*;
    use sunrise_sim::mobs::Species;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    fn check(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        target: Target,
        expected: &[String],
        phase: &str,
        failures: &mut Vec<String>,
    ) {
        let live = game.tags_of(target);
        if live.len() != expected.len() || expected.iter().any(|t| !live.contains(t)) {
            failures.push(format!(
                "{phase}: live count {} expected {}",
                live.len(),
                expected.len()
            ));
        }
        let saved = game
            .command_target_nbt(target)
            .expect("actual entity save view");
        assert_eq!(saved.get("Health"), Some(&Tag::Float(10.0_f32.to_bits())));
        assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
        assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
        assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(17)));
        let valid = match saved.get("Tags") {
            None => expected.is_empty(),
            Some(Tag::List(tags)) => {
                !expected.is_empty()
                    && tags.len() == expected.len()
                    && tags.iter().all(|t| matches!(t, Tag::String(_)))
                    && expected
                        .iter()
                        .all(|s| tags.contains(&Tag::String(s.as_str().into())))
            }
            _ => false,
        };
        if !valid {
            failures.push(format!(
                "{phase}: saved Tags count/type/membership mismatch"
            ));
        }
    }

    fn action(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        command: &str,
        expected: bool,
        failures: &mut Vec<String>,
    ) {
        if game.run_command(command) != expected {
            failures.push(format!("command result: {command}"));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn loaded_pig_tags_cap_before_dedup_and_live_rewrites_adopt_the_set() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert_eq!(
            game.world.block(4, 199, 4),
            game.pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap()
        );
        let start = game.world.game_time;
        let unique: Vec<String> = (0..1025).map(|i| format!("h252_{i:04}")).collect();
        let mut duplicates = vec!["h252_repeat".to_string(); 1024];
        duplicates.push("h252_late".into());
        let small = vec![
            "h252_beta".to_string(),
            "h252_alpha".into(),
            "h252_beta".into(),
        ];
        let encode = |tags: &[String]| {
            format!(
                "[{}]",
                tags.iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let mut failures = Vec::new();
        for (index, (input, expected)) in [
            (Some(encode(&unique)), unique[..1024].to_vec()),
            (Some(encode(&duplicates)), vec!["h252_repeat".to_string()]),
            (
                Some(encode(&small)),
                vec!["h252_alpha".to_string(), "h252_beta".into()],
            ),
            (Some("\"h252_scalar\"".into()), vec![]),
            (None, vec![]),
            (Some("[]".into()), vec![]),
        ]
        .into_iter()
        .enumerate()
        {
            let n = index + 1;
            let uuid = format!("000000fc-0000-0000-0000-{n:012x}");
            let x = 4.5 + index as f64 * 2.0;
            let tags = input.map_or_else(String::new, |s| format!(",Tags:{s}"));
            assert!(game.run_command(&format!(
                "summon pig {x} 200 4.5 {{UUID:[I;252,0,0,{n}],Health:10f,NoAI:1b,NoGravity:1b,Motion:[0d,0d,0d],PortalCooldown:17{tags}}}"
            )));
            let target = game.target_with_uuid([252, 0, 0, n as i32]).unwrap();
            let Target::Mob(id) = target else {
                panic!("actual pig target");
            };
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            assert_eq!(mob.species(), Species::Pig);
            assert_eq!(mob.position, Vec3::new(x, 200.0, 4.5));
            assert_eq!(mob.health, 10.0);
            assert!(mob.no_ai && mob.no_gravity);
            assert_eq!(game.uuid_for(id), [252, 0, 0, n as i32]);
            check(
                &mut game,
                target,
                &expected,
                &format!("load {index}"),
                &mut failures,
            );
            if index == 0 {
                action(
                    &mut game,
                    &format!("tag {uuid} add h252_extra"),
                    false,
                    &mut failures,
                );
                action(
                    &mut game,
                    &format!("tag {uuid} remove h252_0000"),
                    true,
                    &mut failures,
                );
                action(
                    &mut game,
                    &format!("tag {uuid} add h252_extra"),
                    true,
                    &mut failures,
                );
                let mut after = unique[1..1024].to_vec();
                after.push("h252_extra".into());
                check(&mut game, target, &after, "full remove/add", &mut failures);
            } else if index == 1 {
                action(
                    &mut game,
                    &format!("tag {uuid} add h252_added"),
                    true,
                    &mut failures,
                );
                let after = vec!["h252_repeat".to_string(), "h252_added".into()];
                check(
                    &mut game,
                    target,
                    &after,
                    "duplicate load then add",
                    &mut failures,
                );
                action(
                    &mut game,
                    &format!("tag {uuid} remove h252_added"),
                    true,
                    &mut failures,
                );
                check(
                    &mut game,
                    target,
                    &expected,
                    "remove added tag",
                    &mut failures,
                );
            } else if index == 2 {
                action(
                    &mut game,
                    &format!("tag {uuid} add h252_alpha"),
                    false,
                    &mut failures,
                );
                assert!(game.run_command(&format!(
                    "data modify entity {uuid} Tags set value {}",
                    encode(&duplicates)
                )));
                check(
                    &mut game,
                    target,
                    &["h252_repeat".into()],
                    "live rewrite",
                    &mut failures,
                );
            }
            assert_eq!(
                game.mobs.iter().find(|m| m.id == id).unwrap().position,
                Vec3::new(x, 200.0, 4.5)
            );
            assert_eq!(game.uuid_for(id), [252, 0, 0, n as i32]);
            assert_eq!(game.world.game_time, start);
        }
        assert!(
            failures.is_empty(),
            "actual load/getter/save/mutation failures: {failures:#?}"
        );
    }
}

#[cfg(test)]
mod item_display_saved_transform_tests {
    use super::*;
    use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h132-{:?}", std::thread::current().id())),
        );
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
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        for (x, z) in [(0, 1), (1, 0), (1, 1)] {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
        assert!(
            game.run_command("summon pig 13.5 200 2.5 {UUID:[I;132,0,0,99],NoAI:1b,NoGravity:1b}")
        );
        game.world.random.set_seed(132);
        test(&mut game);
    }
    const IDENTITY: &str = "{translation:[0f,0f,0f],left_rotation:[0f,0f,0f,1f],scale:[1f,1f,1f],right_rotation:[0f,0f,0f,1f]}";
    const CHANGED: &str = "{translation:[1.25f,-2.5f,3f],left_rotation:[0f,1f,0f,0f],scale:[2f,3f,4f],right_rotation:[0f,0f,0f,1f]}";
    fn expected(translation: [f32; 3], left: [f32; 4], scale: [f32; 3]) -> Tag {
        let floats = |v: &[f32]| {
            Tag::List(
                v.iter()
                    .map(|x| Tag::Float(if *x == 0.0 { 0 } else { x.to_bits() }))
                    .collect(),
            )
        };
        Tag::Compound(
            [
                ("translation".into(), floats(&translation)),
                ("left_rotation".into(), floats(&left)),
                ("scale".into(), floats(&scale)),
                ("right_rotation".into(), floats(&[0.0, 0.0, 0.0, 1.0])),
            ]
            .into_iter()
            .collect(),
        )
    }
    fn summon(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        n: i32,
        transform: Option<&str>,
    ) -> u32 {
        let suffix = transform
            .map(|t| format!(",transformation:{t}"))
            .unwrap_or_default();
        assert!(game.run_command(&format!("summon item_display 6.5 200 6.5 {{UUID:[I;132,0,0,{n}],Rotation:[12.5f,-20f],NoGravity:1b,CustomName:\"display\",Tags:[\"h132\"],item:{{id:\"minecraft:diamond\",count:2,components:{{\"minecraft:custom_data\":{{h132:7}}}}}}{suffix}}}")));
        let o = game
            .server
            .others
            .0
            .iter()
            .find(|o| game.persist.uuid_of(o.id) == Some([132, 0, 0, n]))
            .unwrap();
        assert_eq!(o.kind, "minecraft:item_display");
        assert_eq!(o.position, Vec3::new(6.5, 200.0, 6.5));
        assert_eq!([o.yaw, o.pitch], [12.5, -20.0]);
        let id = o.id;
        let saved = game.command_target_nbt(Target::Other(id)).unwrap();
        let stack = sunrise_sim::save::item_from_nbt(saved.get("item").unwrap()).unwrap();
        assert_eq!(&*stack.item, "minecraft:diamond");
        assert_eq!(stack.count, 2);
        assert_eq!(
            saved
                .get("item")
                .unwrap()
                .get("components")
                .unwrap()
                .get("minecraft:custom_data")
                .unwrap()
                .get("h132"),
            Some(&Tag::Int(7))
        );
        id
    }
    #[test]
    #[ignore = "daily"]
    fn omitted_item_display_transformation_is_saved_as_typed_identity() {
        with_game(|game| {
            let clock = game.world.game_time;
            let id = summon(game, 1, None);
            let identity = expected([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
            let tag = game.command_target_nbt(Target::Other(id)).unwrap();
            assert_eq!(tag.get("transformation"), Some(&identity));
            let raw = game.persist.raw.remove(&id).unwrap();
            let fresh = game.command_target_nbt(Target::Other(id)).unwrap();
            assert_eq!(fresh.get("transformation"), Some(&identity));
            assert_eq!(fresh.get("UUID"), tag.get("UUID"));
            assert_eq!(fresh.get("Pos"), tag.get("Pos"));
            assert_eq!(fresh.get("interpolation_duration"), Some(&Tag::Int(0)));
            assert_eq!(fresh.get("teleport_duration"), Some(&Tag::Int(0)));
            assert!(fresh.get("start_interpolation").is_none());
            game.persist.raw.insert(id, raw);
            assert_eq!(game.world.game_time, clock);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn admitted_compound_item_transforms_are_actual_saved_state() {
        with_game(|game| {
            let clock = game.world.game_time;
            for (n, text, want) in [
                (
                    2,
                    CHANGED,
                    expected([1.25, -2.5, 3.0], [0.0, 1.0, 0.0, 0.0], [2.0, 3.0, 4.0]),
                ),
                (
                    3,
                    IDENTITY,
                    expected([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
                ),
            ] {
                let id = summon(game, n, Some(text));
                let before = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(before.get("transformation"), Some(&want));
                let raw = game.persist.raw.remove(&id).unwrap();
                let fresh = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(
                    fresh.get("transformation"),
                    Some(&want),
                    "real loaded state survives without raw"
                );
                game.persist.raw.insert(id, raw);
            }
            assert_eq!(game.world.game_time, clock);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn existing_native_item_display_setter_carries_only_transform_state() {
        use mod_api::Transformation;
        with_game(|game| {
            // Same actual server-context setup as existing native machine/contact fixtures.
            game.options.dedicated = Some(Vec::new());
            let id = summon(game, 4, None);
            let uuid = game.persist.uuid_of(id);
            let clock = game.world.game_time;
            let update = Transformation {
                translation: [9.0, 8.0, 7.0],
                left_rotation: [0.0; 4],
                scale: [2.0; 3],
                right_rotation: [0.0, 0.0, 0.0, 2.0],
            };
            let display = game.display(game.entity(id).unwrap()).unwrap();
            assert!(display.set_transformation(game, update));
            assert!(display.set_interpolation_duration(game, 13));
            assert!(display.set_teleport_duration(game, 5));
            assert!(display.set_start_interpolation(game, 7));
            assert!(display.set_shadow_radius(game, 1.25));
            let raw = game.persist.raw.remove(&id).unwrap();
            for field in [
                "transformation",
                "interpolation_duration",
                "teleport_duration",
                "start_interpolation",
            ] {
                assert!(raw.get(field).is_none(), "{field} is resident");
            }
            assert_eq!(display.transformation(game), Some(update));
            assert_eq!(display.interpolation_duration(game), Some(13));
            assert_eq!(display.teleport_duration(game), Some(5));
            assert_eq!(display.start_interpolation(game), Some(7));

            let fresh = game.command_target_nbt(Target::Other(id)).unwrap();
            let floats = |values: &[f32]| {
                Tag::List(
                    values
                        .iter()
                        .map(|value| Tag::Float(value.to_bits()))
                        .collect(),
                )
            };
            let want = Tag::compound([
                ("translation", floats(&[9.0, 8.0, 7.0])),
                ("left_rotation", floats(&[0.0; 4])),
                ("scale", floats(&[2.0; 3])),
                ("right_rotation", floats(&[0.0, 0.0, 0.0, 2.0])),
            ]);
            assert_eq!(fresh.get("transformation"), Some(&want));
            assert_eq!(fresh.get("interpolation_duration"), Some(&Tag::Int(13)));
            assert_eq!(fresh.get("teleport_duration"), Some(&Tag::Int(5)));
            assert!(fresh.get("start_interpolation").is_none());
            assert_eq!(game.persist.uuid_of(id), uuid);
            assert_eq!(game.world.game_time, clock);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn item_display_matrices_and_invalid_values_save_a_typed_transformation() {
        with_game(|game| {
            for (n, form) in [
                (5, "[1f,0f,0f,0f,0f,1f,0f,0f,0f,0f,1f,0f,0f,0f,0f,1f]"),
                (6, "{}"),
                (7, "\"invalid\""),
            ] {
                assert!(game.run_command(&format!("summon item_display 6.5 200 6.5 {{UUID:[I;132,0,0,{n}],transformation:{form},item:{{id:\"minecraft:diamond\",count:1}}}}")));
                let id = game
                    .server
                    .others
                    .0
                    .iter()
                    .find(|o| game.persist.uuid_of(o.id) == Some([132, 0, 0, n]))
                    .unwrap()
                    .id;
                let identity = expected([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
                let saved = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(saved.get("transformation"), Some(&identity), "{form}");
                let raw = game.persist.raw.remove(&id).unwrap();
                let fresh = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(fresh.get("transformation"), Some(&identity), "{form}");
                game.persist.raw.insert(id, raw);
            }
            for (n, kind, extra) in [(8, "marker", ",data:{audit:7}"), (9, "leash_knot", "")] {
                assert!(game.run_command(&format!(
                    "summon {kind} 10.5 200 10.5 {{UUID:[I;132,0,0,{n}]{extra}}}"
                )));
                let id = game
                    .server
                    .others
                    .0
                    .iter()
                    .find(|o| game.persist.uuid_of(o.id) == Some([132, 0, 0, n]))
                    .unwrap()
                    .id;
                assert!(
                    game.command_target_nbt(Target::Other(id))
                        .unwrap()
                        .get("transformation")
                        .is_none()
                );
            }
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| game.persist.uuid_of(m.id) == Some([132, 0, 0, 99]))
                    .unwrap()
                    .health,
                10.0
            );
        });
    }
}

#[cfg(test)]
mod baby_modifier_mutation_tests {
    use super::*;
    use sunrise_sim::mobs::finalize::{self, Attr};
    use sunrise_sim::motion::PlayerMotion;

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("h235-{:?}", std::thread::current().id())),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("difficulty normal"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            test(game);
        });
    }
    fn zombie(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([235, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live zombie with UUID {n}");
        i[0]
    }
    /// The effective movement speed, the stored (permanent) minecraft:baby
    /// amount, and the transient one the model reports.
    fn state(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        n: i32,
    ) -> (f64, Option<f64>, Option<f64>) {
        let m = &game.mobs[zombie(game, n)];
        assert!(
            close(m.movement_speed(), finalize::value(m, Attr::MovementSpeed)),
            "gameplay speed must follow the effective movement attribute"
        );
        (
            finalize::value(m, Attr::MovementSpeed),
            finalize::modifier_amount(m, Attr::MovementSpeed, "minecraft:baby"),
            finalize::transient(m, Attr::MovementSpeed).map(|t| t.0),
        )
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// Expected values from the official 26.3 server: the baby's
    /// minecraft:baby modifier refuses a duplicate add; a removal takes it
    /// away (speed back to the base 0.23) and lets a permanent one be added;
    /// setBaby (an IsBaby merge that changes it) replaces a permanent one
    /// with the transient modifier, and an adult has none.
    #[test]
    #[ignore = "daily"]
    fn the_baby_speed_modifier_answers_add_and_remove_like_any_other() {
        with_game(|game| {
            let attr = "minecraft:movement_speed";
            for (n, baby) in [(1, 1), (2, 1), (3, 0)] {
                assert!(game.run_command(&format!(
                    "summon minecraft:zombie {}.5 200 4.5 {{UUID:[I;235,0,0,{n}],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b,Health:20f,IsBaby:{baby}b}}",
                    3 * n
                )));
            }
            let sel =
                |n: i32| format!("@e[type=minecraft:zombie,nbt={{UUID:[I;235,0,0,{n}]}},limit=1]");
            let mut failures = Vec::new();
            // Setup controls: a baby carries the transient 0.5 (0.345), an adult none.
            let (v, p, t) = state(game, 1);
            assert!(
                close(v, 0.345_000_006_258_487_7) && p.is_none() && t == Some(0.5),
                "baby setup"
            );
            let (v, p, t) = state(game, 3);
            assert!(
                close(v, 0.230_000_004_172_325_13) && p.is_none() && t.is_none(),
                "adult setup"
            );

            if game.run_command(&format!(
                "attribute {} {attr} modifier add minecraft:baby 0.125 add_value",
                sel(1)
            )) {
                failures.push("dup: the add of a held id succeeded".into());
            }
            let (v, p, _) = state(game, 1);
            if !close(v, 0.345_000_006_258_487_7) || p.is_some() {
                failures.push(format!("dup: speed {v}, stored {p:?}"));
            }

            if !game.run_command(&format!(
                "attribute {} {attr} modifier remove minecraft:baby",
                sel(2)
            )) {
                failures.push("remove: the removal failed".into());
            }
            let (v, p, _) = state(game, 2);
            if !close(v, 0.230_000_004_172_325_13) || p.is_some() {
                failures.push(format!("remove: speed {v}, stored {p:?}"));
            }
            if game.run_command(&format!(
                "attribute {} {attr} modifier remove minecraft:baby",
                sel(2)
            )) {
                failures.push("remove: a second removal succeeded".into());
            }
            if !game.run_command(&format!(
                "attribute {} {attr} modifier add minecraft:baby 0.125 add_value",
                sel(2)
            )) {
                failures.push("remove: the add after it failed".into());
            }
            let (v, p, _) = state(game, 2);
            if !close(v, 0.355_000_004_172_325_13) || p != Some(0.125) {
                failures.push(format!("remove then add: speed {v}, stored {p:?}"));
            }

            // The adult: a permanent one, then setBaby(true) and setBaby(false).
            assert!(game.run_command(&format!(
                "attribute {} {attr} modifier add minecraft:baby 0.125 add_value",
                sel(3)
            )));
            assert!(game.run_command(&format!("data merge entity {} {{IsBaby:1b}}", sel(3))));
            let (v, p, _) = state(game, 3);
            if !close(v, 0.345_000_006_258_487_7) || p.is_some() {
                failures.push(format!("perm made a baby: speed {v}, stored {p:?}"));
            }
            assert!(game.run_command(&format!("data merge entity {} {{IsBaby:0b}}", sel(3))));
            let (v, p, _) = state(game, 3);
            if !close(v, 0.230_000_004_172_325_13) || p.is_some() {
                failures.push(format!("perm made an adult: speed {v}, stored {p:?}"));
            }
            assert!(failures.is_empty(), "baby modifier: {failures:#?}");
        });
    }

    /// Added before production, after the baseline: Zombie.setBaby in place
    /// (no /data reload, which drops saved modifiers) removes a permanent
    /// minecraft:baby and, for a baby, adds the transient one; an unrelated
    /// permanent modifier stays.
    #[test]
    #[ignore = "daily"]
    fn set_baby_in_place_replaces_a_permanent_baby_modifier() {
        with_game(|game| {
            assert!(game.run_command(
                "summon minecraft:zombie 4.5 200 8.5 {UUID:[I;235,0,0,9],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b,Health:20f}"
            ));
            let i = zombie(game, 9);
            let m = &mut game.mobs[i];
            finalize::add_modifier(
                m,
                Attr::MovementSpeed,
                "minecraft:baby",
                0.125,
                finalize::Op::AddValue,
            );
            finalize::add_modifier(
                m,
                Attr::MovementSpeed,
                "h235:other",
                0.01,
                finalize::Op::AddValue,
            );
            // Setup control: both permanent ones are held by the adult.
            assert_eq!(
                finalize::modifier_amount(m, Attr::MovementSpeed, "minecraft:baby"),
                Some(0.125)
            );
            let mut failures = Vec::new();
            finalize::set_baby(m, true);
            if finalize::modifier_amount(m, Attr::MovementSpeed, "minecraft:baby").is_some() {
                failures.push("baby: the permanent minecraft:baby stayed".to_string());
            }
            if finalize::transient(m, Attr::MovementSpeed).map(|t| t.0) != Some(0.5) {
                failures.push("baby: no transient minecraft:baby".into());
            }
            if finalize::modifier_amount(m, Attr::MovementSpeed, "h235:other") != Some(0.01) {
                failures.push("baby: the unrelated modifier went".into());
            }
            // (0.23 + 0.01) * 1.5
            if !close(
                finalize::value(m, Attr::MovementSpeed),
                (0.230_000_004_172_325_13 + 0.01) * 1.5,
            ) {
                failures.push(format!(
                    "baby: speed {}",
                    finalize::value(m, Attr::MovementSpeed)
                ));
            }
            finalize::set_baby(m, false);
            if finalize::transient(m, Attr::MovementSpeed).is_some()
                || finalize::modifier_amount(m, Attr::MovementSpeed, "minecraft:baby").is_some()
            {
                failures.push("adult: a minecraft:baby is left".into());
            }
            if !close(
                finalize::value(m, Attr::MovementSpeed),
                0.230_000_004_172_325_13 + 0.01,
            ) {
                failures.push(format!(
                    "adult: speed {}",
                    finalize::value(m, Attr::MovementSpeed)
                ));
            }
            assert!(failures.is_empty(), "in-place setBaby: {failures:#?}");
        });
    }

    /// Added after the baseline (needs the fix's flag): a removal of the
    /// transient minecraft:baby lasts until setBaby; a copy keeps it; a
    /// reload (a /data merge, whose Entity.load runs setBaby) and an
    /// in-place setBaby bring it back; an adult made a baby gets it.
    #[test]
    #[ignore = "daily"]
    fn a_removed_baby_modifier_stays_removed_until_set_baby() {
        with_game(|game| {
            let attr = "minecraft:movement_speed";
            for (n, baby) in [(11, 1), (12, 0)] {
                assert!(game.run_command(&format!(
                    "summon minecraft:zombie {}.5 200 12.5 {{UUID:[I;235,0,0,{n}],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b,Health:20f,IsBaby:{baby}b}}",
                    n - 8
                )));
            }
            let sel =
                |n: i32| format!("@e[type=minecraft:zombie,nbt={{UUID:[I;235,0,0,{n}]}},limit=1]");
            let removed =
                |game: &sunrise_gameplay::game::ServerGame<'_>, n: i32| state(game, n).2.is_none();
            let mut failures = Vec::new();
            assert!(game.run_command(&format!(
                "attribute {} {attr} modifier remove minecraft:baby",
                sel(11)
            )));
            for _ in 0..2 {
                if !removed(game, 11) {
                    failures.push("the removal did not last across reads".to_string());
                }
            }
            let copy = game.mobs[zombie(game, 11)].clone();
            if finalize::transient(&copy, Attr::MovementSpeed).is_some() {
                failures.push("a copy lost the removal".into());
            }
            let i = zombie(game, 11);
            finalize::set_baby(&mut game.mobs[i], true);
            if removed(game, 11) {
                failures.push("in-place setBaby(true) did not bring it back".into());
            }
            assert!(game.run_command(&format!(
                "attribute {} {attr} modifier remove minecraft:baby",
                sel(11)
            )));
            assert!(removed(game, 11));
            assert!(game.run_command(&format!(
                "data merge entity {} {{CustomName:\"h235\"}}",
                sel(11)
            )));
            if removed(game, 11) {
                failures.push("a reload did not bring it back".into());
            }
            // The adult: nothing to remove; made a baby, it has the transient one.
            if game.run_command(&format!(
                "attribute {} {attr} modifier remove minecraft:baby",
                sel(12)
            )) {
                failures.push("adult: a removal succeeded".into());
            }
            let i = zombie(game, 12);
            finalize::set_baby(&mut game.mobs[i], true);
            if state(game, 12).2 != Some(0.5) {
                failures.push("adult made a baby: no transient one".into());
            }
            assert!(
                failures.is_empty(),
                "baby modifier lifecycle: {failures:#?}"
            );
        });
    }
}

#[cfg(test)]
mod attribute_instance_save_tests {
    use super::*;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::motion::PlayerMotion;

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("h105-{:?}", std::thread::current().id())),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            test(game);
        });
    }
    fn mob(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([105, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live mob with UUID {n}");
        i[0]
    }
    /// The ids of the attribute instances the mob's save holds, in order.
    fn saved_ids(game: &mut sunrise_gameplay::game::ServerGame<'_>, i: usize) -> Vec<String> {
        let tag = game
            .command_target_nbt(Target::Mob(game.mobs[i].id))
            .expect("saved mob");
        match tag.get("attributes") {
            Some(Tag::List(l)) => l.iter().filter_map(|t| text(t.get("id"))).collect(),
            _ => Vec::new(),
        }
    }

    /// Expected values from the official 26.3 server: a fresh mob has no
    /// attribute instance (AttributeMap.pack saves []); its first tick makes
    /// movement_speed (aiStep -> removeFrost -> getAttribute); an instance it
    /// was loaded with stays.
    #[test]
    #[ignore = "daily"]
    fn a_mob_saves_the_attribute_instances_it_has() {
        with_game(|game| {
            let speed = || vec!["minecraft:movement_speed".to_string()];
            let mut failures = Vec::new();
            for (kind, n) in [("pig", 1), ("allay", 2)] {
                assert!(game.run_command(&format!(
                    "summon minecraft:{kind} {}.5 200 4.5 {{UUID:[I;105,0,0,{n}],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b}}",
                    2 * n
                )));
            }
            // Before any tick, without the load's compound.
            for (kind, n) in [("pig", 1), ("allay", 2)] {
                let i = mob(game, n);
                game.common.persist.raw.remove(&game.common.mobs[i].id);
                if !saved_ids(game, i).is_empty() {
                    failures.push(format!("{kind} before a tick: {:?}", saved_ids(game, i)));
                }
            }
            game.tick_world(true).unwrap();
            for (kind, n) in [("pig", 1), ("allay", 2)] {
                let i = mob(game, n);
                game.common.persist.raw.remove(&game.common.mobs[i].id);
                if saved_ids(game, i) != speed() {
                    failures.push(format!("{kind} after a tick: {:?}", saved_ids(game, i)));
                }
            }
            // A save of the ticked pig, loaded again: the loaded instance stays
            // before the new pig's first tick.
            let i = mob(game, 1);
            let mut saved = game
                .command_target_nbt(Target::Mob(game.mobs[i].id))
                .unwrap();
            if let Tag::Compound(c) = &mut saved {
                c.insert("UUID".into(), Tag::IntArray(vec![105, 0, 0, 3]));
            }
            game.load_mob(saved, None).unwrap();
            let j = mob(game, 3);
            if saved_ids(game, j) != speed() {
                failures.push(format!(
                    "reloaded pig before a tick: {:?}",
                    saved_ids(game, j)
                ));
            }
            assert!(failures.is_empty(), "attribute instances: {failures:#?}");
        });
    }

    /// Added after the H105 baseline; expectations from the 26.3 source, not
    /// a live Java run. Load-created instances stay (Zombie and Piglin
    /// .setBaby the speed, Rabbit.setVariant the attack damage; an adult
    /// Zoglin's setBaby touches nothing), an unticked mob saves an empty
    /// list, and an instance the mob was loaded with stays before and after
    /// its first tick. Ids are compared as sets: vanilla's order is its
    /// map's.
    #[test]
    #[ignore = "daily"]
    fn load_created_and_loaded_instances_stay_and_an_empty_list_is_saved() {
        with_game(|game| {
            // The hostile summons need a non-Peaceful difficulty.
            assert!(game.run_command("difficulty normal"));
            let set = |v: Vec<String>| {
                let mut v = v;
                v.sort();
                v
            };
            let ids = |v: &[&str]| set(v.iter().map(|s| format!("minecraft:{s}")).collect());
            let fields = "NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b";
            let cases = [
                ("pig", 11, "", ids(&[]), ids(&["movement_speed"])),
                (
                    "zombie",
                    12,
                    "",
                    ids(&["movement_speed"]),
                    ids(&["movement_speed"]),
                ),
                (
                    "piglin",
                    13,
                    ",IsImmuneToZombification:1b",
                    ids(&["movement_speed"]),
                    ids(&["movement_speed"]),
                ),
                (
                    "rabbit",
                    14,
                    "",
                    ids(&["attack_damage"]),
                    ids(&["attack_damage", "movement_speed"]),
                ),
                ("zoglin", 15, "", ids(&[]), ids(&["movement_speed"])),
                (
                    "pig",
                    16,
                    ",attributes:[{id:\"minecraft:scale\",base:1.0d}]",
                    ids(&["scale"]),
                    ids(&["movement_speed", "scale"]),
                ),
            ];
            for (kind, n, extra, _, _) in &cases {
                assert!(game.run_command(&format!(
                    "summon minecraft:{kind} {}.5 200 8.5 {{UUID:[I;105,0,0,{n}],{fields}{extra}}}",
                    n - 9
                )));
            }
            let mut failures = Vec::new();
            for (kind, n, _, before, _) in &cases {
                let i = mob(game, *n);
                // The load's compound stays (the loaded scale instance is in it).
                let got = set(saved_ids(game, i));
                if &got != before {
                    failures.push(format!("{kind} {n} before a tick: {got:?}"));
                }
            }
            let i = mob(game, 11);
            let tag = game
                .command_target_nbt(Target::Mob(game.mobs[i].id))
                .unwrap();
            if tag.get("attributes") != Some(&Tag::List(Vec::new())) {
                failures.push(format!(
                    "pig before a tick: attributes {:?}",
                    tag.get("attributes")
                ));
            }
            game.tick_world(true).unwrap();
            for (kind, n, _, _, after) in &cases {
                let i = mob(game, *n);
                let got = set(saved_ids(game, i));
                if &got != after {
                    failures.push(format!("{kind} {n} after a tick: {got:?}"));
                }
            }
            assert!(failures.is_empty(), "attribute instances: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod mob_inventory_merge_tests {
    use super::*;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::motion::PlayerMotion;

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("h138-{:?}", std::thread::current().id())),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            test(game);
        });
    }
    fn mob(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> usize {
        let i: Vec<usize> = (0..game.mobs.len())
            .filter(|&i| game.persist.uuid_of(game.mobs[i].id) == Some([138, 0, 0, n]))
            .collect();
        assert_eq!(i.len(), 1, "one live mob with UUID {n}");
        i[0]
    }
    /// The live inventory: (item, count, custom_data h138) per stack, in order.
    fn live(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        i: usize,
    ) -> Vec<(String, u32, Option<i32>)> {
        let m = &game.mobs[i];
        let stacks = if m.species() == sunrise_sim::mobs::Species::Villager {
            sunrise_sim::mobs::villager::ext(m)
                .map(|e| e.inventory.clone())
                .unwrap_or_default()
        } else {
            m.extra.inventory.clone()
        };
        stacks
            .iter()
            .map(|s| {
                let saved = sunrise_sim::save::item_to_nbt(s, &Default::default());
                let data = match saved
                    .get("components")
                    .and_then(|c| c.get("minecraft:custom_data"))
                    .and_then(|d| d.get("h138"))
                {
                    Some(Tag::Int(v)) => Some(*v),
                    _ => None,
                };
                (s.item.to_string(), s.count, data)
            })
            .collect()
    }
    /// The saved Inventory, read the same way.
    fn saved(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        i: usize,
    ) -> Vec<(String, u32, Option<i32>)> {
        let tag = game
            .command_target_nbt(Target::Mob(game.mobs[i].id))
            .expect("saved mob");
        let Some(Tag::List(list)) = tag.get("Inventory") else {
            return Vec::new();
        };
        list.iter()
            .filter_map(sunrise_sim::save::item_from_nbt)
            .map(|s| {
                let saved = sunrise_sim::save::item_to_nbt(&s, &Default::default());
                let data = match saved
                    .get("components")
                    .and_then(|c| c.get("minecraft:custom_data"))
                    .and_then(|d| d.get("h138"))
                {
                    Some(Tag::Int(v)) => Some(*v),
                    _ => None,
                };
                (s.item.to_string(), s.count, data)
            })
            .collect()
    }

    /// Expected values from the official 26.3 server: SimpleContainer
    /// .fromItemList adds each saved stack with addItem, which fills earlier
    /// stacks of the same item and components up to the limit, then empty
    /// slots, and drops what does not fit in the eight.
    #[test]
    #[ignore = "daily"]
    fn a_piglin_and_a_villager_merge_their_saved_inventory_stacks() {
        with_game(|game| {
            let it = |id: &str, n: u32| format!("{{id:\"minecraft:{id}\",count:{n}}}");
            let tagged = |n: u32, v: i32| {
                format!(
                    "{{id:\"minecraft:stone\",count:{n},components:{{\"minecraft:custom_data\":{{h138:{v}}}}}}}"
                )
            };
            let s = |id: &str, n: u32, d: Option<i32>| (format!("minecraft:{id}"), n, d);
            let cases: Vec<(&str, Vec<String>, Vec<(String, u32, Option<i32>)>)> = vec![
                (
                    "two_three",
                    vec![it("stone", 2), it("stone", 3)],
                    vec![s("stone", 5, None)],
                ),
                (
                    "forty",
                    vec![it("stone", 40), it("stone", 40)],
                    vec![s("stone", 64, None), s("stone", 16, None)],
                ),
                (
                    "pearls",
                    vec![it("ender_pearl", 10), it("ender_pearl", 10)],
                    vec![s("ender_pearl", 16, None), s("ender_pearl", 4, None)],
                ),
                (
                    "interleaved",
                    vec![it("stone", 40), it("dirt", 1), it("stone", 40)],
                    vec![
                        s("stone", 64, None),
                        s("dirt", 1, None),
                        s("stone", 16, None),
                    ],
                ),
                (
                    "components",
                    vec![tagged(2, 1), tagged(3, 2)],
                    vec![s("stone", 2, Some(1)), s("stone", 3, Some(2))],
                ),
                (
                    "same_components",
                    vec![tagged(2, 1), tagged(3, 1)],
                    vec![s("stone", 5, Some(1))],
                ),
                (
                    "different",
                    vec![it("stone", 2), it("dirt", 3)],
                    vec![s("stone", 2, None), s("dirt", 3, None)],
                ),
                (
                    "nine_full",
                    vec![it("stone", 64); 9],
                    vec![s("stone", 64, None); 8],
                ),
            ];
            let mut failures = Vec::new();
            let mut n = 0;
            for kind in ["piglin", "villager"] {
                for (name, items, want) in &cases {
                    n += 1;
                    let extra = if kind == "piglin" {
                        ",IsImmuneToZombification:1b"
                    } else {
                        ""
                    };
                    assert!(game.run_command(&format!(
                        "summon minecraft:{kind} 4.5 200 4.5 {{UUID:[I;138,0,0,{n}],NoAI:1b,PersistenceRequired:1b,Silent:1b{extra},Inventory:[{}]}}",
                        items.join(",")
                    )));
                    let i = mob(game, n);
                    // No raw echo: the load's compound is gone, only the model is left.
                    game.common.persist.raw.remove(&game.common.mobs[i].id);
                    if &live(game, i) != want {
                        failures.push(format!("{kind} {name}: live {:?}", live(game, i)));
                    }
                    if &saved(game, i) != want {
                        failures.push(format!("{kind} {name}: saved {:?}", saved(game, i)));
                    }
                }
            }
            // The model is what saves: a change to it shows in the next save.
            let i = mob(game, 1);
            game.mobs[i].extra.inventory[0].count = 7;
            if saved(game, i) != vec![s("stone", 7, None)] {
                failures.push(format!("piglin mutation: saved {:?}", saved(game, i)));
            }
            assert!(failures.is_empty(), "mob inventory merge: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod invalid_passenger_tests {
    use super::*;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_sim::motion::PlayerMotion;

    const PIG: &str = "NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b";
    const UNKNOWN: &str = "{id:\"minecraft:h161_unknown\",Tags:[\"h161_unknown_entry\"]}";
    const NO_ID: &str = "{Tags:[\"h161_no_id_entry\"]}";

    fn pig(tag: &str, n: i32, extra: &str) -> String {
        format!("{{id:\"minecraft:pig\",UUID:[I;161,0,0,{n}],Tags:[\"{tag}\"],{PIG}{extra}}}")
    }
    fn summon(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        tag: &str,
        n: i32,
        passengers: &str,
    ) -> u32 {
        assert!(game.run_command(&format!(
            "summon minecraft:pig 4.5 200 4.5 {{UUID:[I;161,0,0,{n}],Tags:[\"{tag}\"],{PIG},Passengers:{passengers}}}"
        )));
        mob(game, n)
    }
    /// The mob in the level with UUID `n`.
    fn mob(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> u32 {
        let ids: Vec<u32> = game
            .mobs
            .iter()
            .filter(|m| game.persist.uuid_of(m.id) == Some([161, 0, 0, n]))
            .map(|m| m.id)
            .collect();
        assert_eq!(ids.len(), 1, "one live mob with UUID {n}");
        ids[0]
    }
    fn mount(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Option<u32> {
        game.mobs.iter().find(|m| m.id == id).unwrap().extra.mount
    }
    /// Each saved passenger of `id` as (id, UUID word 3, first tag), in order;
    /// `None` when the entity saves no Passengers.
    fn saved(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        id: u32,
    ) -> Option<Vec<(String, Option<i32>, String)>> {
        let tag = game.command_target_nbt(Target::Mob(id)).expect("saved mob");
        let Some(Tag::List(list)) = tag.get("Passengers") else {
            return None;
        };
        let text = |t: Option<&Tag>| match t {
            Some(Tag::String(s)) => s.to_string().unwrap_or_default(),
            _ => String::new(),
        };
        Some(
            list.iter()
                .map(|p| {
                    let uuid = match p.get("UUID") {
                        Some(Tag::IntArray(u)) if u.len() == 4 => Some(u[3]),
                        _ => None,
                    };
                    let first = match p.get("Tags") {
                        Some(Tag::List(t)) => text(t.first()),
                        _ => String::new(),
                    };
                    (text(p.get("id")), uuid, first)
                })
                .collect(),
        )
    }
    fn pig_entry(n: i32, tag: &str) -> (String, Option<i32>, String) {
        ("minecraft:pig".into(), Some(n), tag.into())
    }

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("h161-{:?}", std::thread::current().id())),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            let time = game.world.game_time;
            test(game);
            assert_eq!(game.world.game_time, time);
        });
    }

    /// Expected values from the official 26.3 server:
    /// EntityType.loadPassengersRecursive keeps only the entries that load,
    /// and Entity.saveWithoutId saves the riders the entity has.
    #[test]
    #[ignore = "daily"]
    fn passengers_that_do_not_load_are_not_saved() {
        with_game(|game| {
            let mut failures = Vec::new();
            for (name, n, passengers) in [
                ("unknown", 102, format!("[{UNKNOWN}]")),
                ("missing_id", 103, format!("[{NO_ID}]")),
                ("noncompound", 104, "[1,2]".to_string()),
            ] {
                let id = summon(game, &format!("h161_{name}"), n, &passengers);
                let got = saved(game, id);
                if got.is_some() {
                    failures.push(format!("{name}: saved Passengers {got:?}"));
                }
            }
            if game.mobs.len() != 3 {
                failures.push(format!(
                    "invalid entries: {} mobs, expected 3",
                    game.mobs.len()
                ));
            }

            // Valid riders keep their order and their own state.
            let passengers = format!(
                "[{UNKNOWN},{},{NO_ID},{}]",
                pig("h161_mixed_a", 201, ",CustomName:\"a\""),
                pig("h161_mixed_b", 202, "")
            );
            let root = summon(game, "h161_mixed", 105, &passengers);
            let [a, b] = [201, 202].map(|n| mob(game, n));
            if mount(game, a) != Some(root) || mount(game, b) != Some(root) {
                failures.push("mixed: a valid rider does not ride the root".into());
            }
            let got = saved(game, root);
            let want = vec![
                pig_entry(201, "h161_mixed_a"),
                pig_entry(202, "h161_mixed_b"),
            ];
            if got.as_ref() != Some(&want) {
                failures.push(format!("mixed: saved {got:?}"));
            }
            let a_saved = game.command_target_nbt(Target::Mob(a)).unwrap();
            if !matches!(a_saved.get("CustomName"), Some(Tag::String(s)) if s.to_string().unwrap_or_default() == "a")
            {
                failures.push("mixed: rider a lost its CustomName".into());
            }

            // A rider's own invalid entries go too.
            let passengers = format!(
                "[{}]",
                pig(
                    "h161_nested_child",
                    203,
                    &format!(",Passengers:[{UNKNOWN}]")
                )
            );
            let root = summon(game, "h161_nested", 106, &passengers);
            let child = mob(game, 203);
            if saved(game, root) != Some(vec![pig_entry(203, "h161_nested_child")]) {
                failures.push(format!("nested: root saved {:?}", saved(game, root)));
            }
            if saved(game, child).is_some() {
                failures.push(format!("nested: child saved {:?}", saved(game, child)));
            }

            // Without a kept compound the tree writer has only the live riders.
            let root = mob(game, 105);
            let kept = game.persist.raw.remove(&root);
            if saved(game, root) != Some(want.clone()) {
                failures.push(format!("mixed, no raw: saved {:?}", saved(game, root)));
            }
            if let Some(kept) = kept {
                game.persist.raw.insert(root, kept);
            }

            // Entity.load does not read Passengers: a merge leaves the riders.
            let passengers = format!("[{UNKNOWN},{}]", pig("h161_merged_child", 220, ""));
            let root = summon(game, "h161_merged", 120, &passengers);
            let child = mob(game, 220);
            let merged = |game: &mut sunrise_gameplay::game::ServerGame<'_>, merge: &str| {
                game.run_command(&format!(
                    "data merge entity @e[tag=h161_merged,limit=1] {merge}"
                ))
            };
            let passengers_only = format!("{{Passengers:[{UNKNOWN},{NO_ID}]}}");
            let rider = Some(vec![pig_entry(220, "h161_merged_child")]);
            // An unrelated field applies; the second, identical merge changes
            // nothing (DataCommands: ERROR_MERGE_UNCHANGED) and keeps the raw and the mount.
            for (merge, expected) in [
                ("{CustomName:\"merged2\"}", true),
                ("{CustomName:\"merged2\"}", false),
            ] {
                let raw = game.persist.raw.get(&root).cloned();
                let ok = merged(game, merge);
                if ok != expected {
                    failures.push(format!("merged {merge}: result {ok}, expected {expected}"));
                }
                if !expected && game.persist.raw.get(&root) != raw.as_ref() {
                    failures.push(format!("merged {merge}: a refused merge changed the raw"));
                }
                if saved(game, root) != rider {
                    failures.push(format!("merged {merge}: saved {:?}", saved(game, root)));
                }
                if mount(game, child) != Some(root) {
                    failures.push(format!("merged {merge}: child left"));
                }
            }
            let name = game.command_target_nbt(Target::Mob(root)).unwrap();
            if !matches!(name.get("CustomName"), Some(Tag::String(s)) if s.to_string().unwrap_or_default() == "merged2")
            {
                failures.push("merged: CustomName did not apply".into());
            }
            // Passenger-only merges, again and again: nothing accumulates.
            for n in 0..3 {
                if !merged(game, &passengers_only) {
                    failures.push(format!("passengers-only merge {n}: failed"));
                }
                if saved(game, root) != rider {
                    failures.push(format!(
                        "passengers-only merge {n}: saved {:?}",
                        saved(game, root)
                    ));
                }
                if mount(game, child) != Some(root) {
                    failures.push(format!("passengers-only merge {n}: child left"));
                }
            }
            assert!(failures.is_empty(), "invalid passengers: {failures:#?}");
        });
    }
}

#[cfg(test)]
#[path = "persist/summon_uuid_conflict_tests.rs"]
mod summon_uuid_conflict_tests;

#[cfg(test)]
#[path = "persist/thrown_item_tests.rs"]
pub(crate) mod thrown_item_tests;

#[cfg(test)]
mod h262_enderman_absolute_anger_tests {
    use super::*;
    use sunrise_core::random::RandomSource;
    use sunrise_core::{geometry::Vec3, position::ChunkPos};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };
    pub(super) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h262-{:?}", std::thread::current().id())),
        );
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

        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            &mut game,
            ChunkPos::new(0, 0),
        );
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
        assert!(
            game.run_command("summon pig 13.5 200 2.5 {UUID:[I;262,0,0,99],NoAI:1b,NoGravity:1b}")
        );
        game.world.random.set_seed(262);
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn load(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        n: i32,
        end: Option<i64>,
    ) -> (u32, usize) {
        let suffix = end
            .map(|v| format!(",anger_end_time:{v}L"))
            .unwrap_or_default();
        let cmd = format!(
            "summon enderman 6.5 200 4.5 {{UUID:[I;262,1,0,{n}],Health:40f,NoAI:1b,NoGravity:1b,Motion:[0d,0d,0d],Tags:[h262],CustomName:\"end\"{suffix}}}"
        );
        assert!(game.run_command(&cmd));
        let id = game
            .persist
            .uuids
            .iter()
            .find_map(|(id, u)| (*u == [262, 1, 0, n]).then_some(*id))
            .unwrap();
        let i = game.mobs.iter().position(|m| m.id == id).unwrap();
        assert_eq!(game.mobs[i].species(), sunrise_sim::mobs::Species::Enderman);
        assert!(game.mobs[i].no_ai);
        assert_eq!(game.mobs[i].health, 40.0);
        assert_eq!(game.mobs[i].position, Vec3::new(6.5, 200.0, 4.5));
        assert_eq!(game.mobs[i].extra.anger_end, end.unwrap_or(-1));
        (id, i)
    }
    fn check(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        i: usize,
        expected: i64,
        label: &str,
        failures: &mut Vec<String>,
    ) {
        let saved = game.mob_tree_nbt(i);
        if saved.get("anger_end_time") != Some(&Tag::Long(expected)) {
            failures.push(format!(
                "{label}: expected {expected}L, actual {:?}",
                saved.get("anger_end_time")
            ));
        }
        assert_eq!(
            saved.get("UUID"),
            Some(&Tag::IntArray(
                game.persist.uuids[&game.mobs[i].id].to_vec()
            ))
        );
        // Tags follow the existing raw-field policy; a deliberately no-raw
        // subject has no synthetic source of Tags. Preserve them when present.
        if let Some(tags) = game
            .persist
            .raw
            .get(&game.mobs[i].id)
            .and_then(|raw| raw.get("Tags"))
        {
            assert_eq!(saved.get("Tags"), Some(tags));
        }
        assert!(saved.get("CustomName").is_some());
    }
    #[test]
    #[ignore = "daily"]
    fn h262_loaded_noai_enderman_actual_ticks_and_model_projection() {
        with_game(|game| {
            game.world.game_time = 100;
            assert!(game.run_command("gamerule spawn_mobs false"));
            let (id, i) = load(game, 1, Some(999999));
            let age = game.mobs[i].age;
            let mut failures = Vec::new();
            for tick in 0..=2 {
                if tick > 0 {
                    game.world.game_time += 1;
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(6, 4));
                    game.tick_creature_entities().unwrap();
                }
                assert_eq!(game.mobs[i].id, id);
                assert_eq!(game.mobs[i].age, age + tick);
                assert_eq!(game.world.game_time, 100 + i64::from(tick));
                assert_eq!(game.mobs[i].position, Vec3::new(6.5, 200.0, 4.5));
                assert_eq!(game.mobs[i].velocity, Vec3::ZERO);
                check(game, i, 999999, "loaded/ticked", &mut failures);
                check(game, i, 999999, "repeated", &mut failures);
            }
            game.persist.raw.remove(&id);
            game.mobs[i].extra.anger_end = 777777;
            check(game, i, 777777, "no raw/model mutation", &mut failures);
            game.persist.raw.insert(
                id,
                Tag::compound([
                    ("anger_end_time", Tag::Long(123)),
                    ("retained", Tag::Int(17)),
                ]),
            );
            game.mobs[i].extra.anger_end = 888888;
            check(
                game,
                i,
                888888,
                "contradictory raw/model mutation",
                &mut failures,
            );
            assert_eq!(game.mob_tree_nbt(i).get("retained"), Some(&Tag::Int(17)));
            assert!(failures.is_empty(), "{failures:#?}");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn h262_noai_enderman_absent_zero_past_and_future_are_actual_longs() {
        with_game(|game| {
            game.world.game_time = 100;
            let mut failures = Vec::new();
            for (n, end) in [None, Some(0), Some(-5), Some(1), Some(999999)]
                .into_iter()
                .enumerate()
            {
                let (_, i) = load(game, n as i32 + 2, end);
                check(
                    game,
                    i,
                    end.unwrap_or(-1),
                    "typed loaded boundary",
                    &mut failures,
                );
            }
            assert!(failures.is_empty(), "{failures:#?}");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn h262_live_enderman_write_uses_actual_decoded_timestamp() {
        with_game(|game| {
            game.world.game_time = 100;
            let (id, i) = load(game, 9, Some(999999));
            let age = game.mobs[i].age;
            assert!(game.run_command("data modify entity 00000106-0000-0001-0000-000000000009 anger_end_time set value 777777L"));
            assert_eq!(
                game.mobs[i].extra.anger_end, 777777,
                "actual adoption, before saved read"
            );
            assert_eq!(game.mobs[i].id, id);
            assert_eq!(game.mobs[i].age, age);
            assert_eq!(game.world.game_time, 100);
            assert!(game.mobs[i].ai.target.is_none());
            let mut failures = Vec::new();
            check(game, i, 777777, "live write", &mut failures);
            assert!(failures.is_empty(), "{failures:#?}");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn h262_existing_ai_and_other_neutral_counter_projection_is_unchanged() {
        with_game(|game| {
            game.world.game_time = 100;
            let (_, i) = load(game, 20, Some(999999));
            game.mobs[i].no_ai = false;
            game.mobs[i].extra.anger = 50;
            assert_eq!(
                game.mob_tree_nbt(i).get("anger_end_time"),
                Some(&Tag::Long(150))
            );
            assert!(
                game.run_command(
                    "summon wolf 8.5 200 4.5 {UUID:[I;262,1,0,21],NoAI:1b,NoGravity:1b}"
                )
            );
            let i = game
                .mobs
                .iter()
                .position(|m| m.species() == sunrise_sim::mobs::Species::Wolf)
                .unwrap();
            game.mobs[i].extra.anger = 50;
            assert_eq!(
                game.mob_tree_nbt(i).get("anger_end_time"),
                Some(&Tag::Long(150))
            );
            game.world.game_time += 1;
            assert_eq!(
                game.mob_tree_nbt(i).get("anger_end_time"),
                Some(&Tag::Long(150)) // an absolute end (#13076)
            );
        });
    }
}

#[cfg(test)]
mod sulfur_body_attribute_tests {
    use super::*;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h192-{:?}", std::thread::current().id())),
        );
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("scoreboard objectives add h192 dummy"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    /// `/attribute <cube> <attribute> get 1000`, read back through a score.
    fn read(game: &mut sunrise_gameplay::game::ServerGame<'_>, n: i32, attribute: &str) -> i32 {
        let sel = format!("@e[type=minecraft:sulfur_cube,nbt={{UUID:[I;192,0,0,{n}]}},limit=1]");
        assert!(game.run_command(&format!(
            "execute store result score #v h192 run attribute {sel} minecraft:{attribute} get 1000"
        )));
        (-3000..=3000)
            .find(|v| game.run_command_quietly(&format!("execute if score #v h192 matches {v}")))
            .unwrap_or(i32::MIN)
    }

    /// Expected values from the official 26.3 server: a sulfur cube reads its
    /// defaults until a tick adopts its body (collectEquipmentChanges); then the
    /// body's archetype modifiers count (oak planks: bouncy).
    #[test]
    #[ignore = "daily"]
    fn a_sulfur_cube_reads_its_body_archetype_modifiers_after_a_tick() {
        with_game(|game| {
            for (n, body) in [
                (1, ",equipment:{body:{id:\"minecraft:oak_planks\"}}"),
                (2, ""),
            ] {
                assert!(game.run_command(&format!(
                        "summon minecraft:sulfur_cube {}.5 200 4.5 {{UUID:[I;192,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1{body}}}",
                        3 * n
                    )));
            }
            let attrs = [
                "knockback_resistance",
                "explosion_knockback_resistance",
                "bounciness",
                "friction_modifier",
                "air_drag_modifier",
            ];
            let defaults = [0, 0, 0, 1000, 1000];
            let mut failures = Vec::new();
            for (n, label) in [(1, "body"), (2, "none")] {
                for (a, d) in attrs.iter().zip(defaults) {
                    let got = read(game, n, a);
                    if got != d {
                        failures.push(format!("{label} {a} before a tick: {got}, expected {d}"));
                    }
                }
            }
            game.tick_world(true).unwrap();
            game.tick_world(true).unwrap();
            // Bouncy: -2, -2 clamped to 0, +0.9, x0.3, x0.01.
            let bouncy = [-2000, 0, 899, 300, 9];
            for (n, label, want) in [(1, "body", bouncy), (2, "none", defaults)] {
                for (a, d) in attrs.iter().zip(want) {
                    let got = read(game, n, a);
                    if got != d {
                        failures.push(format!("{label} {a} after two ticks: {got}, expected {d}"));
                    }
                }
            }
            assert!(failures.is_empty(), "sulfur body attributes: {failures:#?}");
        });
    }

    /// Added before production (routing compatibility): a cube with no body
    /// keeps the existing effect of a command-set knockback base and permanent
    /// modifier on the cached resistance its physics reads
    /// (Mob::knockback_resistance falls back to attrs.knockback_resistance).
    #[test]
    #[ignore = "daily"]
    fn a_bodiless_cube_keeps_its_command_knockback_resistance_effect() {
        with_game(|game| {
            assert!(game.run_command(
                    "summon minecraft:sulfur_cube 4.5 200 8.5 {UUID:[I;192,0,0,9],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1}"
                ));
            let sel = "@e[type=minecraft:sulfur_cube,nbt={UUID:[I;192,0,0,9]},limit=1]";
            let cached = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let m = game
                    .mobs
                    .iter()
                    .find(|m| game.persist.uuid_of(m.id) == Some([192, 0, 0, 9]))
                    .unwrap();
                (m.attrs.knockback_resistance, m.knockback_resistance())
            };
            let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
            let mut failures = Vec::new();
            for (command, want) in [
                (
                    format!("attribute {sel} minecraft:knockback_resistance base set 0.5"),
                    0.5,
                ),
                (
                    format!(
                        "attribute {sel} minecraft:knockback_resistance modifier add h192:extra 0.25 add_value"
                    ),
                    0.75,
                ),
                (
                    format!(
                        "attribute {sel} minecraft:knockback_resistance modifier remove h192:extra"
                    ),
                    0.5,
                ),
            ] {
                assert!(game.run_command(&command), "{command}");
                let (c, e) = cached(game);
                if !close(c, want) || !close(e, want) {
                    failures.push(format!(
                        "{command}: cached {c}, effective {e}, expected {want}"
                    ));
                }
                if read(game, 9, "knockback_resistance") != (want * 1000.0) as i32 {
                    failures.push(format!(
                        "{command}: /attribute get {}",
                        read(game, 9, "knockback_resistance")
                    ));
                }
            }
            game.tick_world(true).unwrap();
            let (c, _) = cached(game);
            if !close(c, 0.5) {
                failures.push(format!("after a tick: cached {c}"));
            }
            assert!(
                failures.is_empty(),
                "bodiless cube knockback: {failures:#?}"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_readded_archetype_id_updates_the_bodiless_physics_cache() {
        with_game(|game| {
            assert!(game.run_command(
                "summon minecraft:sulfur_cube 4.5 200 8.5 {UUID:[I;192,0,0,19],NoAI:1b,NoGravity:1b,Size:1}"
            ));
            let sel = "@e[type=minecraft:sulfur_cube,nbt={UUID:[I;192,0,0,19]},limit=1]";
            let id = "minecraft:bouncy_add_knockback_resistance";
            assert!(game.run_command(&format!(
                "attribute {sel} minecraft:knockback_resistance modifier add {id} 0.2 add_value"
            )));
            let mob = game
                .mobs
                .iter_mut()
                .find(|m| m.species() == sunrise_sim::mobs::Species::SulfurCube)
                .unwrap();
            mob.extra.set_body(Some("minecraft:oak_planks".into()));
            sunrise_sim::mobs::sulfur_cube::adopt_body(mob);
            mob.extra.set_body(None);
            sunrise_sim::mobs::sulfur_cube::adopt_body(mob);
            assert!(game.run_command(&format!(
                "attribute {sel} minecraft:knockback_resistance modifier add {id} 0.6 add_value"
            )));
            assert_eq!(read(game, 19, "knockback_resistance"), 600);
            // A genuine duplicate is still refused by the authoritative map.
            game.run_command(&format!(
                "attribute {sel} minecraft:knockback_resistance modifier add {id} 0.9 add_value"
            ));
            assert_eq!(read(game, 19, "knockback_resistance"), 600);
            let mob = game
                .mobs
                .iter()
                .find(|m| m.species() == sunrise_sim::mobs::Species::SulfurCube)
                .unwrap();
            assert!((mob.knockback_resistance() - 0.6).abs() < 1e-9);
        });
    }

    /// Added control (H192 routing mirror): base reset and modifier add/remove
    /// on a bodiless and a bodied cube reach the cached resistance, which never
    /// takes the body archetype's transient modifiers (bouncy: -2).
    #[test]
    #[ignore = "daily"]
    fn command_knockback_changes_reach_the_cache_without_archetype_values() {
        with_game(|game| {
            for (n, body) in [
                (10, ""),
                (11, ",equipment:{body:{id:\"minecraft:oak_planks\"}}"),
            ] {
                assert!(game.run_command(&format!(
                        "summon minecraft:sulfur_cube {}.5 200 8.5 {{UUID:[I;192,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Size:1{body}}}",
                        n - 8
                    )));
            }
            game.tick_world(true).unwrap();
            let cached = |game: &sunrise_gameplay::game::ServerGame<'_>, n: i32| {
                game.mobs
                    .iter()
                    .find(|m| game.persist.uuid_of(m.id) == Some([192, 0, 0, n]))
                    .unwrap()
                    .attrs
                    .knockback_resistance
            };
            let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
            let mut failures = Vec::new();
            // (command, cached, /attribute get for the bodied cube)
            for (command, want, bodied) in [
                ("base set 0.5", 0.5, -1500),
                ("modifier add h192:extra 0.25 add_value", 0.75, -1250),
                ("modifier remove h192:extra", 0.5, -1500),
                ("base reset", 0.0, -2000),
                ("modifier add h192:extra 0.25 add_value", 0.25, -1750),
                ("modifier remove h192:extra", 0.0, -2000),
            ] {
                for n in [10, 11] {
                    let sel = format!(
                        "@e[type=minecraft:sulfur_cube,nbt={{UUID:[I;192,0,0,{n}]}},limit=1]"
                    );
                    let full = format!("attribute {sel} minecraft:knockback_resistance {command}");
                    assert!(game.run_command(&full), "{full}");
                    let c = cached(game, n);
                    if !close(c, want) {
                        failures.push(format!("{n} {command}: cached {c}, expected {want}"));
                    }
                }
                let got = read(game, 10, "knockback_resistance");
                if got != (want * 1000.0) as i32 {
                    failures.push(format!("10 {command}: /attribute get {got}"));
                }
                let got = read(game, 11, "knockback_resistance");
                if got != bodied {
                    failures.push(format!(
                        "11 {command}: /attribute get {got}, expected {bodied}"
                    ));
                }
            }
            assert!(failures.is_empty(), "knockback mirror: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod summoned_knot_tests {
    use super::*;
    use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h129-{:?}", std::thread::current().id())),
        );
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

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        for (x, z) in [(0, 1), (1, 0), (1, 1)] {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
        assert!(
            game.run_command("summon pig 13.5 200 2.5 {UUID:[I;129,0,0,99],NoAI:1b,NoGravity:1b}")
        );
        game.world.random.set_seed(129);
        game.load_ticking_neighbourhood(0, 0);
        test(&mut game);
    }
    fn knot_id(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> u32 {
        game.server
            .others
            .0
            .iter()
            .find(|o| game.persist.uuid_of(o.id) == Some([129, 0, 0, n]))
            .unwrap()
            .id
    }
    fn summon_knots(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> [u32; 3] {
        for x in [6, 14] {
            assert!(game.run_command(&format!("setblock {x} 200 6 oak_fence strict")));
        }
        for (n, x) in [(1, 6), (2, 10), (3, 14)] {
            assert!(game.run_command(&format!("summon leash_knot {x}.2 200.8 6.9 {{UUID:[I;129,0,0,{n}],CustomName:\"knot\",Tags:[\"h129\"]}}")));
        }
        [1, 2, 3].map(|n| knot_id(game, n))
    }
    #[test]
    #[ignore = "daily"]
    fn summoned_knots_adopt_normalized_position_and_save_common_defaults() {
        with_game(|game| {
            let clock = game.world.game_time;
            let ids = summon_knots(game);
            for (i, id) in ids.into_iter().enumerate() {
                let o = game.server.others.0.iter().find(|o| o.id == id).unwrap();
                assert_eq!(o.kind, "minecraft:leash_knot");
                assert_eq!(game.persist.uuid_of(id), Some([129, 0, 0, i as i32 + 1]));
                let expected = Vec3::new(6.5 + 4.0 * i as f64, 200.375, 6.5);
                assert_eq!(
                    o.position, expected,
                    "actual loaded model, not a getter-only answer"
                );
                let tag = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(
                    tag.get("Pos"),
                    Some(&Tag::List(
                        expected
                            .components()
                            .map(|v| Tag::Double(v.to_bits()))
                            .to_vec()
                    ))
                );
                for (key, value) in [
                    ("Air", Tag::Short(300)),
                    ("Invulnerable", Tag::Byte(0)),
                    ("OnGround", Tag::Byte(0)),
                    ("PortalCooldown", Tag::Int(0)),
                    ("fall_distance", Tag::Double(0.0f64.to_bits())),
                ] {
                    assert_eq!(tag.get(key), Some(&value), "{key}");
                }
                assert_eq!(
                    tag.get("Tags"),
                    Some(&Tag::List(vec![Tag::String("h129".into())]))
                );
                // The same real model, with no raw source, must still save defaults.
                let raw = game.persist.raw.remove(&id).unwrap();
                let fresh = game.command_target_nbt(Target::Other(id)).unwrap();
                assert_eq!(fresh.get("Air"), Some(&Tag::Short(300)));
                assert_eq!(fresh.get("Pos"), tag.get("Pos"));
                game.persist.raw.insert(id, raw);
            }
            assert_eq!(game.world.game_time, clock);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn summoned_knots_check_actual_fence_support_on_tick_101() {
        with_game(|game| {
            let clock = game.world.game_time;
            let ids = summon_knots(game);
            assert!(
                game.run_command(
                    "summon marker 8.2 200.8 10.9 {UUID:[I;129,0,0,4],data:{audit:7}}"
                )
            );
            let marker = game
                .server
                .others
                .0
                .iter()
                .find(|o| o.kind == "minecraft:marker")
                .unwrap()
                .id;
            let marker_before = game.command_target_nbt(Target::Other(marker)).unwrap();
            for tick in 1..=101 {
                game.tick().unwrap();
                assert_eq!(game.world.game_time, clock + tick);
                if tick == 1 {
                    assert!(game.run_command("setblock 14 200 6 air strict"));
                }
                if tick <= 100 {
                    assert!(
                        ids.iter()
                            .all(|id| game.server.others.0.iter().any(|o| o.id == *id)),
                        "alive through tick{tick}"
                    );
                }
            }
            assert!(
                game.server.others.0.iter().any(|o| o.id == ids[0]),
                "actual fence survives"
            );
            assert!(
                !game.server.others.0.iter().any(|o| o.id == ids[1]),
                "unsupported actual entity gone at101"
            );
            assert!(
                !game.server.others.0.iter().any(|o| o.id == ids[2]),
                "removed support actual entity gone at101"
            );
            assert_eq!(
                game.command_target_nbt(Target::Other(marker)).unwrap(),
                marker_before
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| game.persist.uuid_of(m.id) == Some([129, 0, 0, 99]))
                    .unwrap()
                    .health,
                10.0
            );
            assert!(game.command_target_nbt(Target::Other(ids[1])).is_none());
            assert!(game.run_command("kill @e[type=leash_knot]"));
            assert!(
                !game
                    .server
                    .others
                    .0
                    .iter()
                    .any(|o| o.kind == "minecraft:leash_knot")
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn negative_fractional_knot_position_uses_floor_before_centering() {
        with_game(|game| {
            // Chunk (-1, -1) is already loaded by the fixture's neighbourhood.
            assert!(game.world.chunks.contains_key(&(-1, -1)));
            assert!(game.run_command("setblock -2 199 -3 stone strict"));
            assert!(game.run_command("setblock -2 200 -3 oak_fence strict"));
            assert!(game.run_command("summon leash_knot -1.2 200.8 -2.1 {UUID:[I;129,0,0,5]}"));
            let id = knot_id(game, 5);
            assert_eq!(
                game.server
                    .others
                    .0
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap()
                    .position,
                Vec3::new(-1.5, 200.375, -2.5)
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn explicit_knot_check_counts_only_running_entity_ticking_turns() {
        with_game(|game| {
            let ids = summon_knots(game);
            let clock = game.world.game_time;
            for _ in 0..100 {
                game.tick().unwrap();
            }
            assert!(
                ids.iter()
                    .all(|id| game.server.others.0.iter().any(|o| o.id == *id))
            );
            assert!(game.run_command("tick freeze"));
            for _ in 0..3 {
                game.tick().unwrap();
            }
            assert_eq!(game.world.game_time, clock + 100);
            assert!(
                ids.iter()
                    .all(|id| game.server.others.0.iter().any(|o| o.id == *id))
            );
            assert!(game.run_command("gamerule spectators_generate_chunks false"));
            assert!(game.run_command("gamemode spectator @s"));
            assert!(game.run_command("forceload remove all"));
            assert!(game.run_command("tick unfreeze"));
            for _ in 0..3 {
                game.tick().unwrap();
                assert!(!game.entity_ticking_area()(10, 6));
            }
            assert_eq!(game.world.game_time, clock + 103);
            assert!(
                ids.iter()
                    .all(|id| game.server.others.0.iter().any(|o| o.id == *id))
            );
            assert!(game.run_command("gamemode creative @s"));
            assert!(game.run_command("forceload add 0 0"));
            game.tick().unwrap();
            assert!(game.entity_ticking_area()(10, 6));
            assert!(game.server.others.0.iter().any(|o| o.id == ids[0]));
            assert!(
                !game.server.others.0.iter().any(|o| o.id == ids[1]),
                "101 eligible ticks, not elapsed world time"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn explicit_knot_removal_preserves_a_real_lead_created_knot() {
        with_game(|game| {
            use sunrise_sim::mobs::leash::Holder;
            assert!(game.run_command("tp @s 12.5 200 2.5"));
            assert!(game.run_command("item replace entity @s weapon.mainhand with lead"));
            assert!(game.run_command("setblock 14 200 2 oak_fence strict"));
            let pig = game
                .mobs
                .iter()
                .position(|m| game.persist.uuid_of(m.id) == Some([129, 0, 0, 99]))
                .unwrap();
            assert!(game.interact_leash(pig, "minecraft:lead"));
            assert!(game.use_knot([14, 200, 2], false));
            assert_eq!(game.mobs[pig].extra.leash, Some(Holder::Knot([14, 200, 2])));
            game.lan_assign_ids();
            let natural = game.server.knot_ids[&[14, 200, 2]];
            let ids = summon_knots(game);
            for _ in 0..101 {
                game.tick().unwrap();
            }
            game.lan_assign_ids();
            assert_eq!(game.server.knot_ids[&[14, 200, 2]], natural);
            let pig = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([129, 0, 0, 99]))
                .unwrap();
            assert_eq!(pig.extra.leash, Some(Holder::Knot([14, 200, 2])));
            assert!(!game.server.others.0.iter().any(|o| o.id == ids[1]));
        });
    }
}

#[cfg(test)]
mod camel_pose_saved_tests {
    use super::*;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn loaded_camel_pose_timestamps_survive_actual_save_and_tick() {
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
        options.game_dir = Some(std::env::current_dir().unwrap().join("game"));
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
        let mut chunk = Chunk::empty(0, 0);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        // A mob only ticks with its whole 5x5 chunk neighbourhood loaded.
        for x in -2..=2 {
            for z in -2..=2 {
                if (x, z) != (0, 0) {
                    game.world
                        .chunks
                        .insert((x, z), Arc::new(Chunk::empty(x, z)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(x, z))
                        .unwrap();
                }
            }
        }
        assert!(game.run_command("forceload add -32 -32 32 32"));
        assert!(game.run_command("tick freeze"));
        let initial = game.world.game_time;
        assert!(game.run_command("tick step 128"));
        for _ in 0..128 {
            game.tick_server(false).unwrap();
        }
        let now = game.world.game_time;
        assert_eq!(now, initial + 128);
        let cases = [
            None,
            Some(0),
            Some(now - 24),
            Some(-(now - 24)),
            Some(now + 20),
        ];
        let mut actors = Vec::new();
        for (n, value) in cases.into_iter().enumerate() {
            let number = n as i32 + 1;
            let field = value
                .map(|value| format!(",LastPoseTick:{value}L"))
                .unwrap_or_default();
            assert!(game.run_command(&format!("summon camel {} 200 4.5 {{UUID:[I;94,0,0,{number}],NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Health:32f,Temper:17,Tags:[\"h094\"]{field}}}", 2.5 + n as f64 * 2.5)));
            let mob = game
                .mobs
                .iter()
                .find(|m| game.persist.uuids.get(&m.id) == Some(&[94, 0, 0, number]))
                .unwrap();
            // Prove the real decoder adopted the value before reading saved NBT.
            assert_eq!(mob.extra.pose_change_tick, value.unwrap_or(0));
            actors.push((mob.id, value.unwrap_or(0)));
        }
        for elapsed in 0..=1 {
            assert_eq!(game.world.game_time, now + elapsed);
            for &(id, value) in &actors {
                let index = game.mobs.iter().position(|m| m.id == id).unwrap();
                let mob = &game.mobs[index];
                assert!(mob.no_ai && mob.alive());
                assert_eq!(mob.age, elapsed as u32, "actual entity tick admission");
                assert_eq!(mob.health, 32.0);
                assert_eq!(sunrise_sim::mobs::camel::sitting(mob), value < 0);
                assert_eq!(
                    sunrise_sim::mobs::camel::refuse_to_move(mob),
                    value < 0 || value == now - 24 || value == now + 20
                );
                let saved = game.mob_tree_nbt(index);
                assert_eq!(
                    saved.get("LastPoseTick"),
                    Some(&Tag::Long(value)),
                    "loaded timestamp {value} at delta {elapsed}"
                );
                assert_eq!(
                    saved.get("Tags"),
                    Some(&Tag::List(vec![Tag::String("h094".into())]))
                );
                // The authoritative Game save must also work without source NBT.
                let raw = game.persist.raw.remove(&id);
                assert_eq!(
                    game.mob_tree_nbt(index).get("LastPoseTick"),
                    Some(&Tag::Long(value))
                );
                if let Some(raw) = raw {
                    game.persist.raw.insert(id, raw);
                }
            }
            if elapsed == 0 {
                assert!(game.run_command("tick step 1"));
                game.tick_server(false).unwrap();
            }
        }
        // Unload and reload the chunk, then let time pass: the saved game time
        // stays what it was (`Camel.addAdditionalSaveData` writes it unchanged).
        let written: Vec<Tag> = game
            .entity_chunks(&HashSet::from([(0, 0)]), true)
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .collect();
        game.world
            .loaded_entities
            .push(((0, 0), written.into_iter().map(Into::into).collect()));
        game.spawn_loaded_entities();
        for step in 0..8 {
            for number in 1..=cases.len() as i32 {
                let index = game
                    .mobs
                    .iter()
                    .position(|m| game.persist.uuids.get(&m.id) == Some(&[94, 0, 0, number]))
                    .unwrap();
                let value = actors[number as usize - 1].1;
                assert_eq!(
                    game.mob_tree_nbt(index).get("LastPoseTick"),
                    Some(&Tag::Long(value)),
                    "reloaded timestamp {value} after {step} steps"
                );
            }
            game.tick_server(false).unwrap();
        }
    }
}

#[cfg(test)]
mod mob_hurt_cooldown_tests {
    use super::*;
    use sunrise_sim::motion::PlayerMotion;

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join(format!("h248-{:?}", std::thread::current().id())),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("difficulty normal"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
            game.load_ticking_neighbourhood(0, 0);
            game.creative = true;
            test(game);
        });
    }
    /// The pig with `[I;0,248,0,n]`: its health and its hurt cooldown state
    /// (`invulnerableTime`, `lastHurt`).
    fn pig(game: &sunrise_gameplay::game::ServerGame<'_>, n: i32) -> (f32, i32, f32) {
        let m = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([0, 248, 0, n]))
            .unwrap();
        (m.health, m.invulnerable, m.last_hurt)
    }
    fn summon(game: &mut sunrise_gameplay::game::ServerGame<'_>, n: i32, x: i32) -> String {
        assert!(game.run_command(&format!(
            "summon minecraft:pig {x}.5 200 4.5 {{UUID:[I;0,248,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,attributes:[{{id:\"minecraft:max_health\",base:40.0d}}],Health:40f}}"
        )));
        format!("@e[type=minecraft:pig,nbt={{UUID:[I;0,248,0,{n}]}},limit=1]")
    }

    /// Expected values from the official 26.3 server (LivingEntity.hurtServer):
    /// inside the cooldown a hit is compared, and lastHurt kept, by its amount
    /// before armor and Resistance; actuallyHurt reduces only the difference.
    #[test]
    #[ignore = "daily"]
    fn the_hurt_cooldown_compares_the_incoming_damage_before_its_reductions() {
        with_game(|game| {
            let full = summon(game, 1, 2);
            assert!(game.run_command(&format!(
                "effect give {full} minecraft:resistance 30 4 true"
            )));
            let armored = summon(game, 2, 8);
            assert!(game.run_command(&format!("attribute {armored} minecraft:armor base set 10")));
            let mut failures = Vec::new();
            let check = |failures: &mut Vec<String>,
                         label: &str,
                         got: (f32, i32, f32),
                         want: (f32, i32, f32)| {
                if (got.0 - want.0).abs() >= 1e-4 || got.1 != want.1 || got.2 != want.2 {
                    failures.push(format!(
                        "{label}: (health, invulnerable, last_hurt) {got:?}, expected {want:?}"
                    ));
                }
            };
            // Full Resistance: every hit loses no health.
            let accepted = game.run_command(&format!("damage {full} 1 minecraft:generic"));
            check(
                &mut failures,
                &format!("full 1 accepted {accepted}"),
                pig(game, 1),
                (40.0, 20, 1.0),
            );
            let accepted = game.run_command(&format!("damage {full} 2 minecraft:generic"));
            if !accepted {
                failures.push("full: the stronger hit 2 was refused".into());
            }
            check(&mut failures, "full 2", pig(game, 1), (40.0, 20, 2.0));
            let accepted = game.run_command(&format!("damage {full} 2 minecraft:generic"));
            if accepted {
                failures.push("full: the equal hit 2 was taken".into());
            }
            check(&mut failures, "full equal 2", pig(game, 1), (40.0, 20, 2.0));
            // Armor 10 (cactus is not #bypasses_armor): 4 then 8 deal 2.72 each.
            assert!(game.run_command(&format!("damage {armored} 4 minecraft:cactus")));
            check(&mut failures, "armor 4", pig(game, 2), (37.28, 20, 4.0));
            assert!(game.run_command(&format!("damage {armored} 8 minecraft:cactus")));
            check(&mut failures, "armor 8", pig(game, 2), (34.56, 20, 8.0));
            assert!(failures.is_empty(), "hurt cooldown: {failures:#?}");
        });
    }
}

#[cfg(test)]
mod restored_vehicle_transfer_tests {
    use super::*;
    use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentType, Attachments, Holder};

    #[test]
    #[ignore = "daily"]
    fn runtime_transfers_do_not_alias_a_saved_player_vehicle_with_the_same_uuid() {
        sunrise_gameplay::game::native_entity_refs::tests::with_resident_game(|game| {
            let runtime = AttachmentType {
                namespace: "test",
                name: "runtime",
                default: || 0,
                codec: None,
                copy_on_death: false,
                sync_codec: None,
            };
            let saved = AttachmentType {
                namespace: "test",
                name: "saved",
                default: || 0,
                codec: Some(AttachmentCodec {
                    encode: |n: &i32| Ok([("value".into(), Tag::Int(*n))].into()),
                    decode: |c| {
                        Ok(match c.get(&"value".into()) {
                            Some(Tag::Int(n)) => *n,
                            _ => {
                                return Err(
                                    sunrise_mod_runtime::attachments::AttachmentError::Decode,
                                );
                            }
                        })
                    },
                }),
                copy_on_death: false,
                sync_codec: None,
            };
            let tag = |kind: &str, n: i32| {
                Tag::compound([
                    ("id", Tag::String(kind.into())),
                    ("UUID", Tag::IntArray(vec![n, 2, 3, 4])),
                    (
                        "Pos",
                        Tag::List(vec![
                            Tag::Double(1.0f64.to_bits()),
                            Tag::Double(70.0f64.to_bits()),
                            Tag::Double(1.0f64.to_bits()),
                        ]),
                    ),
                ])
            };
            for generic in [false, true] {
                let n = if generic { 720 } else { 710 };
                let uuid = [n, 2, 3, 4];
                // Capture a real source before loading the destination's saved vehicle.
                let source = game.load_mob(tag("minecraft:cow", n), None).unwrap();
                let source_child = (!generic).then(|| {
                    game.load_mob(tag("minecraft:pig", n + 1), Some(source))
                        .unwrap()
                });
                let mut map = Attachments::default();
                runtime.set(&mut map, Holder::Level, 41).unwrap();
                saved.set(&mut map, Holder::Level, 51).unwrap();
                game.entity_attachments.insert(source, map);
                let index = game.mobs.iter().position(|m| m.id == source).unwrap();
                let incoming = game.mob_tree_nbt(index);
                let tree = (!generic).then(|| game.capture_vehicle_tree_state(source).unwrap());
                let state = generic.then(|| game.take_transferred_entity_state(source));
                game.mobs
                    .retain(|m| m.id != source && Some(m.id) != source_child);
                game.detach_entity(source);
                if let Some(child) = source_child {
                    game.detach_entity(child);
                }

                let mut destination_tag = tag("minecraft:camel", n);
                if let Tag::Compound(c) = &mut destination_tag {
                    c.insert(
                        "Passengers".into(),
                        Tag::List(vec![tag("minecraft:sheep", n + 2)]),
                    );
                }
                let wrapper = Tag::compound([(
                    "RootVehicle",
                    Tag::compound([("Entity", destination_tag.clone())]),
                )]);
                let destination = game.restore_saved_vehicle(uuid, Some(&wrapper)).unwrap();
                assert_eq!(game.restored_vehicle(uuid), Some(destination));
                let destination_child = game.mob_passenger_ids(destination)[0];
                let original_rider = game.mob_object(destination).unwrap().extra.rider;
                let mut map = Attachments::default();
                runtime.set(&mut map, Holder::Level, 42).unwrap();
                saved.set(&mut map, Holder::Level, 52).unwrap();
                game.entity_attachments.insert(destination, map);
                // The ordinary saved-player/chunk disk duplicate still coalesces.
                assert_eq!(
                    game.load_mob(destination_tag.clone(), None),
                    Some(destination)
                );
                let before = game.server.next_entity;
                game.spawn_loaded_entity((0, 0), destination_tag, None);
                assert_eq!(game.server.next_entity, before);

                let arrived = if generic {
                    game.spawn_loaded_entity((0, 0), incoming, state);
                    game.persist
                        .off_level
                        .iter()
                        .find_map(|(&id, _)| (game.persist.uuid_of(id) == Some(uuid)).then_some(id))
                        .unwrap()
                } else {
                    let restored = game
                        .restore_vehicle_tree_state(incoming, tree.unwrap())
                        .unwrap();
                    assert_ne!(restored.by_source[&source], destination);
                    assert_eq!(restored.by_source[&source], restored.root);
                    let child = restored.by_source[&source_child.unwrap()];
                    assert_eq!(
                        game.mob_object(child).unwrap().extra.mount,
                        Some(restored.root)
                    );
                    restored.root
                };
                assert_ne!(arrived, destination);
                assert_ne!(arrived, source);
                assert!(
                    game.entity(arrived).is_none(),
                    "ordinary duplicate UUID admission still refuses residency"
                );
                assert!(game.entity(destination).is_some());
                assert_eq!(
                    game.mob_object(arrived).unwrap().species(),
                    sunrise_sim::mobs::Species::Cow
                );
                assert_eq!(
                    game.mob_object(destination).unwrap().extra.rider,
                    original_rider
                );
                assert_eq!(
                    game.mob_object(destination_child).unwrap().extra.mount,
                    Some(destination)
                );
                assert_eq!(game.mob_passenger_ids(destination), vec![destination_child]);
                for (id, r, s) in [(arrived, 41, 51), (destination, 42, 52)] {
                    let map = game.entity_attachments.get_mut(&id).unwrap();
                    assert_eq!(runtime.get(map, Holder::Level), Ok(r));
                    assert_eq!(saved.get(map, Holder::Level), Ok(s));
                }
            }
        });
    }
}

/// TraderLlama.maybeDespawn discards the llama in the tick its delay runs out
/// (H182): it leaves the level's entity list in that tick.
#[cfg(test)]
mod trader_llama_discard_tests {
    use super::*;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h182-{:?}", std::thread::current().id())),
        );
        // A fixed name: no generated sunrise-player.txt in the working directory.
        options.player_name = Some("H182".into());
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        game.load_ticking_neighbourhood(0, 0);
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("difficulty normal"));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        test(&mut game);
    }

    /// Expected values from the official 26.3 server: a delay of 5 is gone
    /// after the fifth tick and a delay of 0 after the first; a tame and a
    /// persistent llama stay.
    #[test]
    #[ignore = "daily"]
    fn a_trader_llama_leaves_the_level_in_the_tick_its_delay_runs_out() {
        with_game(|game| {
            for (n, extra) in [
                (1, "DespawnDelay:5"),
                (2, "DespawnDelay:0"),
                (3, "DespawnDelay:5,Tame:1b"),
                (4, "DespawnDelay:5,PersistenceRequired:1b"),
            ] {
                assert!(game.run_command(&format!(
                    "summon minecraft:trader_llama {}.5 200 8.5 {{UUID:[I;0,182,0,{n}],NoAI:0b,Silent:1b,{extra}}}",
                    2 + 3 * n
                )));
            }
            let present = |game: &sunrise_gameplay::game::ServerGame<'_>| -> [bool; 4] {
                [1, 2, 3, 4].map(|n| {
                    game.mobs.iter().any(|m| {
                        game.persist.uuid_of(m.id) == Some([0, 182, 0, n])
                            && &*m.kind.id == "minecraft:trader_llama"
                    })
                })
            };
            let mut failures = Vec::new();
            for tick in 1..=6 {
                game.tick_world(true).unwrap();
                let want = [tick < 5, false, true, true];
                let got = present(game);
                if got != want {
                    failures.push(format!(
                        "after tick {tick}: in the list {got:?}, expected {want:?}"
                    ));
                }
            }
            assert!(failures.is_empty(), "trader llama discard: {failures:#?}");
        });
    }
}
#[cfg(test)]
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[path = "persist/object_memory.rs"]
mod object_memory;
