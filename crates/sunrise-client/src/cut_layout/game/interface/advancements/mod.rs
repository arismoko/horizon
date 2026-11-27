#![cfg(test)]
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::server_side::advancements::View;
use sunrise_items::items::ItemStack;
use sunrise_sim::advancements as adv;
use sunrise_sim::advancements::Tree;
#[cfg(test)]
#[path = "inventory_changed_tests.rs"]
pub(crate) mod inventory_changed_tests;

#[cfg(test)]
pub(crate) mod progress_tests;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn helmet_wear_reports_the_old_stack_and_the_unclamped_damage() {
        const ID: &str = "testmod:wear";
        let scratch = std::env::temp_dir().join(format!("helmet-wear-{}", std::process::id()));
        let dir = scratch.join("data/testmod/advancement");
        std::fs::create_dir_all(&dir).unwrap();
        // An iron helmet has 165 points. At 164, a hit of 8 takes 2: damage 166, one past the
        // maximum, so durability -1, and a delta of -2 from the damage the stack had.
        std::fs::write(
            dir.join("wear.json"),
            r#"{
            "criteria": {
                "worn": {
                    "trigger": "minecraft:item_durability_changed",
                    "conditions": {
                        "durability": {"min": -1, "max": -1},
                        "delta": {"min": -2, "max": -2}
                    }
                }
            },
            "requirements": [["worn"]]
        }"#,
        )
        .unwrap();
        super::super::native_entity_refs::tests::with_game(|game| {
            game.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
            let origin = sunrise_core::geometry::Vec3::ZERO;
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, origin).unwrap());
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            helmet.damage = 164;
            game.armor[0] = Some(helmet);
            let source = sunrise_sim::health::NO_ATTACK.player_source("minecraft:falling_anvil", 0);
            game.hurt_helmet(&source, 8.0);
            assert_eq!(game.armor[0], None);
            assert!(game.adv.progress.has(ID, "worn"));
        });
        std::fs::remove_dir_all(scratch).unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn mob_name_components_read_live_text_without_saved_nbt() {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::commands_common::Target,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
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
        let kind = game.res.mob_types()["minecraft:pig"].clone();
        game.mobs.push(sunrise_sim::mobs::Mob::new(
            12500,
            kind,
            Vec3::new(4.5, 200.0, 4.5),
            0.0,
        ));
        let matches = |game: &sunrise_gameplay::game::ServerGame<'_>, key: &str, want: Value| {
            let view = game.entity_view_of(Target::Mob(12500));
            assert_eq!(view.kind, "minecraft:pig");
            assert!(view.nbt.is_none());
            let mut predicate = serde_json::Map::new();
            predicate.insert(
                key.into(),
                serde_json::json!({"minecraft:custom_name": want}),
            );
            adv::entity(
                &Value::Object(predicate),
                &view,
                &View(game),
                [4.5, 200.0, 4.5],
            )
        };
        for key in ["components", "minecraft:components"] {
            assert!(!matches(&game, key, serde_json::json!("Bob")));
        }
        for (name, unequal) in [
            (serde_json::json!("Bob"), serde_json::json!("minecraft:Bob")),
            (serde_json::json!("minecraft:Bob"), serde_json::json!("Bob")),
            (
                serde_json::json!({"text": "Bob", "color": "red", "bold": true}),
                serde_json::json!("Bob"),
            ),
            (
                serde_json::json!({"text": "B", "extra": [{"text": "ob"}]}),
                serde_json::json!("Bob"),
            ),
        ] {
            let text = sunrise_game_data::scoreboard::comp_text(&name);
            game.mobs[0].custom_name = Some(text.plain());
            game.mobs[0].custom_name_text = Some(text);
            for key in ["components", "minecraft:components"] {
                assert!(matches(&game, key, name.clone()), "{key}: {name}");
                assert!(!matches(&game, key, unequal.clone()), "{key}: {unequal}");
            }
        }
        game.mobs[0].custom_name = None;
        game.mobs[0].custom_name_text = None;
        for key in ["components", "minecraft:components"] {
            assert!(!matches(&game, key, serde_json::json!("Bob")));
        }
    }
    #[test]
    #[ignore = "daily"]
    fn mob_predicate_view_reads_active_effect_instances() {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::commands_common::Target,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
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
        let kind = game.res.mob_types()["minecraft:zombie"].clone();
        let actor = |id| {
            let mut mob =
                sunrise_sim::mobs::Mob::new(id, kind.clone(), Vec3::new(4.5, 200.0, 4.5), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            mob
        };
        game.mobs.push(actor(32300));
        let matches = |game: &sunrise_gameplay::game::ServerGame<'_>, id, effects: Value| {
            let view = game.entity_view_of(Target::Mob(id));
            assert_eq!(view.kind, "minecraft:zombie");
            adv::entity(
                &serde_json::json!({"minecraft:effects": effects}),
                &view,
                &View(game),
                [4.5, 200.0, 4.5],
            )
        };
        let speed_two = serde_json::json!({"minecraft:speed": {"amplifier": 2}});
        assert!(!matches(&game, 32300, speed_two.clone()));
        game.mobs[0].add_effect_with("minecraft:speed", -1, 2, false, false);
        assert_eq!(
            game.mobs[0].extra.effects,
            vec![sunrise_sim::mobs::StoredEffect::new(
                "minecraft:speed",
                -1,
                2
            )]
        );
        assert_eq!(
            game.entity_view_of(Target::Mob(32300)).effects,
            vec![("minecraft:speed".to_owned(), 2, -1)]
        );
        assert!(matches(&game, 32300, speed_two));
        assert!(!matches(
            &game,
            32300,
            serde_json::json!({"minecraft:speed": {"amplifier": 5}})
        ));
        game.mobs[0].add_effect("minecraft:fire_resistance", 400, 0);
        assert!(matches(
            &game,
            32300,
            serde_json::json!({
                "minecraft:speed": {"amplifier": 2},
                "minecraft:fire_resistance": {"amplifier": 0, "duration": 400}
            })
        ));
        let mut finite = actor(32301);
        finite.add_effect("minecraft:speed", 400, 0);
        game.mobs.push(finite);
        assert!(matches(
            &game,
            32301,
            serde_json::json!({
                "minecraft:speed": {"amplifier": 0, "duration": 400}
            })
        ));
        assert!(!matches(
            &game,
            32301,
            serde_json::json!({
                "minecraft:speed": {"amplifier": 0, "duration": {"max": 399}}
            })
        ));
    }
}

#[cfg(test)]
pub(crate) mod structure_tag_tests {
    use serde_json::json;
    use sunrise_gameplay::game::server_side::advancements::View;
    use sunrise_sim::advancements::{World, id_matches};
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    /// `LocationPredicate.structures` names a `#tag`: the game's own
    /// `worldgen/structure` tags, and with a world's data packs theirs over
    /// the game's.
    #[test]
    #[ignore = "daily"]
    fn location_structure_tags_hold_their_structures() {
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
        let set = |s: &str| json!(s);
        let holds = |game: &sunrise_gameplay::game::ServerGame<'_>, tag: &str, id: &str| {
            id_matches(&set(tag), "worldgen/structure", id, &View(game))
        };
        // A world without packs: the game's own tags.
        assert!(game.server.world_tables.0.is_none());
        assert!(holds(
            &game,
            "#minecraft:village",
            "minecraft:village_taiga"
        ));
        assert!(holds(&game, "#shipwreck", "minecraft:shipwreck_beached"));
        assert!(!holds(&game, "#minecraft:village", "minecraft:shipwreck"));
        assert!(!View(&game).in_tag("worldgen/structure", "mypack:village", "minecraft:igloo"));

        // A pack's own tag (nesting the game's) and an addition to the game's.
        let dir =
            std::env::temp_dir().join(format!("sunrise-structure-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks").join("mypack");
        let tags = pack.join("data/mypack/tags/worldgen/structure");
        let game_tags = pack.join("data/minecraft/tags/worldgen/structure");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::create_dir_all(&game_tags).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            tags.join("village.json"),
            r##"{"values": ["#minecraft:village", "minecraft:igloo"]}"##,
        )
        .unwrap();
        std::fs::write(
            game_tags.join("shipwreck.json"),
            r#"{"values": ["minecraft:ocean_ruin_cold"]}"#,
        )
        .unwrap();
        let settings = json!({"enabled": ["file/mypack"], "disabled": []});
        std::fs::write(dir.join("datapacks.json"), settings.to_string()).unwrap();
        game.server.world_root = dir.clone();
        let stack = sunrise_gameplay::game::functions::pack_stack(
            &dir,
            &root,
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            &Default::default(),
            false,
            &[],
            &[],
        );
        game.server.world_tables.0 = game
            .stage_world_tables(&stack, false, &root)
            .unwrap()
            .tables;
        assert!(game.server.world_tables.0.is_some());
        assert!(holds(&game, "#mypack:village", "minecraft:igloo"));
        assert!(holds(&game, "#mypack:village", "minecraft:village_plains"));
        assert!(!holds(&game, "#mypack:village", "minecraft:shipwreck"));
        assert!(holds(
            &game,
            "#minecraft:shipwreck",
            "minecraft:ocean_ruin_cold"
        ));
        assert!(holds(&game, "#minecraft:shipwreck", "minecraft:shipwreck"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
pub(crate) mod random_chance_command_tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::commands_common::Target;
    #[test]
    #[ignore = "daily"]
    fn registered_named_random_chance_uses_real_source_and_world_draws() {
        use std::time::Instant;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
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

        let dir = std::env::temp_dir().join(format!("sunrise-h122-unit-{}", std::process::id()));
        assert!(!dir.exists());
        let pack = dir.join("datapacks/chance");
        let folder = pack.join("data/h122/predicate");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(pack.join("pack.mcmeta"),r#"{"pack":{"description":"Chance","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
        let function = pack.join("data/h122/function");
        std::fs::create_dir_all(&function).unwrap();
        std::fs::write(
            function.join("admit.mcfunction"),
            "scoreboard players set pack h122 17\n",
        )
        .unwrap();
        let rows: Vec<Value> = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/parity/random-chance-seeded.jsonl"
        ))
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
        for row in &rows {
            std::fs::write(
                folder.join(format!("case_{}.json", row["case"])),
                row["input"].to_string(),
            )
            .unwrap();
        }
        std::fs::write(folder.join("pig.json"),r#"{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:pig"}}"#).unwrap();
        game.server.world_root = dir.clone();
        game.reload_datapacks(false, &[]).unwrap();
        assert!(game.run_command("scoreboard objectives add h122 dummy"));
        assert!(game.run_command("function h122:admit"));
        assert_eq!(game.score.board.score("pack", "h122").unwrap().value, 17);
        let tag = Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(vec![122, 0, 0, 1])),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(4.5_f64.to_bits()),
                    Tag::Double(200.0_f64.to_bits()),
                    Tag::Double(4.5_f64.to_bits()),
                ]),
            ),
            ("Health", Tag::Float(10_f32.to_bits())),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
        ]);
        let id = game.load_mob(tag, None).unwrap();
        assert_eq!(game.persist.uuids.get(&id), Some(&[122, 0, 0, 1]));
        assert_eq!(game.entity_view_of(Target::Mob(id)).kind, "minecraft:pig");
        let clock = game.world.game_time;
        assert!(game.run_command("execute as 0000007a-0000-0000-0000-000000000001 if predicate h122:pig run scoreboard players set identity h122 1"));
        assert_eq!(game.score.board.score("identity", "h122").unwrap().value, 1);
        for row in &rows {
            let seed = row["seed"].as_i64().unwrap();
            let name = format!("h122:case_{}", row["case"]);
            assert!(game.server.fns.0.store.predicates.contains_key(&name));
            game.world.random.set_seed(seed);
            game.run_command("scoreboard players set branch h122 -99");
            game.server.exec.0 = Default::default();
            let source = game.player_source();
            game.execute_line(&format!("execute as 0000007a-0000-0000-0000-000000000001 if predicate {name} run scoreboard players set branch h122 1"),source).unwrap();
            assert_eq!(
                game.score.board.score("branch", "h122").unwrap().value,
                if row["result"].as_bool().unwrap() {
                    1
                } else {
                    -99
                },
                "{row}"
            );
            let continuation: Vec<i64> = (0..4).map(|_| game.world.random.next_long()).collect();
            let expected: Vec<i64> = row["continuation"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            assert_eq!(continuation, expected, "actual world RNG {row}");
        }
        assert_eq!(game.world.game_time, clock);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
pub(crate) mod custom_data_equipment_tests {
    use super::*;
    use sunrise_core::nbt::{Tag, parse_snbt};
    use sunrise_core::position::BlockPos;
    use sunrise_sim::{mobs::finalize, save};

    fn item_predicate(pattern: &str) -> Value {
        serde_json::json!({"items":"minecraft:stick","count":1,
            "predicates":{"minecraft:custom_data":pattern}})
    }
    fn condition(pattern: &str) -> Value {
        serde_json::json!({"type":"minecraft:entity_properties","entity":"this",
            "predicate":{"minecraft:entity_type":"minecraft:pig",
                "minecraft:equipment":{"mainhand":item_predicate(pattern)}}})
    }

    #[test]
    #[ignore = "daily"]
    fn actual_mob_equipment_projects_typed_custom_data_into_named_predicate_evaluation() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
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
            let clock = game.world.game_time;
            let rich = r#"{h289:1b,nested:{k:7,extra:9},entries:[{k:1b,extra:2b},{k:2b}],wide:9007199254740993L,spare:"keep"}"#;
            let mut failures = Vec::new();
            for (i, data) in [(1, Some(rich)), (2, None)] {
                let uuid = [289, 1, 0, i];
                let actor = format!("00000121-0000-0001-0000-{i:012x}");
                let components = data.map_or(String::new(), |d| {
                    format!(",components:{{\"minecraft:custom_data\":{d}}}")
                });
                let x = f64::from(i) + 0.5;
                assert!(game.run_command(&format!(
                "summon minecraft:pig {x} 200 4.5 {{UUID:[I;289,1,0,{i}],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b,Health:10f,equipment:{{mainhand:{{id:\"minecraft:stick\",count:1{components}}}}}}}"
            )));
                assert!(game.world.loaded(i, 4));
                assert_eq!(
                    &*game.res.facts().get(game.world.block(i, 199, 4)).name,
                    "minecraft:stone"
                );
                assert!(game.run_command(&format!("execute as {actor} if entity @s[type=pig,nbt={{UUID:[I;289,1,0,{i}],NoAI:1b,Health:10f,Pos:[{x}d,200d,4.5d]}}]")));
                let id = *game
                    .persist
                    .uuids
                    .iter()
                    .find(|(_, value)| **value == uuid)
                    .expect("actual admitted UUID")
                    .0;
                let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
                assert!(mob.no_ai);
                let stack = finalize::item_stack(mob, finalize::MAINHAND)
                    .expect("actual loaded equipment stack");
                assert_eq!((&*stack.item, stack.count), ("minecraft:stick", 1));
                let saved = save::item_to_nbt(&stack, &Default::default());
                if let Some(data) = data {
                    let (expected, used) = parse_snbt(data).unwrap();
                    assert_eq!(used, data.len());
                    assert_eq!(
                        saved
                            .get("components")
                            .and_then(|t| t.get("minecraft:custom_data")),
                        Some(&expected)
                    );
                    assert_eq!(
                        stack.custom_data().unwrap().get(&"h289".into()),
                        Some(&Tag::Byte(1))
                    );
                    assert_eq!(
                        stack.custom_data().unwrap().get(&"wide".into()),
                        Some(&Tag::Long(9_007_199_254_740_993))
                    );
                    // Contradictory retained raw equipment must not replace actual stack state.
                    let (wrong, _) = parse_snbt(r#"{equipment:{mainhand:{id:"minecraft:stick",count:1,components:{"minecraft:custom_data":{h289:2b}}}}}"#).unwrap();
                    let raw = game
                        .persist
                        .raw
                        .entry(id)
                        .or_insert_with(|| Tag::Compound(Default::default()));
                    let Tag::Compound(raw) = raw else {
                        panic!("actual raw compound")
                    };
                    raw.insert("equipment".into(), wrong.get("equipment").unwrap().clone());
                    assert!(game.run_command(&format!("execute if data entity {actor} equipment.mainhand.components{{\"minecraft:custom_data\":{{h289:1b}}}}")));
                } else {
                    assert!(stack.custom_data().is_none());
                    assert!(game.run_command(&format!("execute unless data entity {actor} equipment.mainhand.components.\"minecraft:custom_data\"")));
                }
                let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
                let view = game.mob_entity(mob);
                assert_eq!(
                    (
                        &*view.equipment[0].as_ref().unwrap().id,
                        view.equipment[0].as_ref().unwrap().count
                    ),
                    ("minecraft:stick", 1)
                );
                let probes: Vec<(&str, bool)> = if data.is_some() {
                    vec![
                        ("{h289:1b}", true),
                        ("{nested:{k:7}}", true),
                        ("{entries:[{k:2b}]}", true),
                        ("{entries:[{k:2b},{k:1b}]}", true),
                        ("{wide:9007199254740993L}", true),
                        ("{}", true),
                        ("{h289:2b}", false),
                        ("{h289:1}", false),
                        ("{missing:1b}", false),
                        ("{entries:[]}", false),
                        ("{wide:9007199254740992L}", false),
                    ]
                } else {
                    vec![("{}", true), ("{h289:1b}", false)]
                };
                for (pattern, want) in probes {
                    let actual =
                        game.conditions_hold(&condition(pattern), Some(&view), [x, 200.0, 4.5]);
                    if actual != want {
                        failures.push((i, pattern, actual, want));
                    }
                }
                // Existing non-equipment views are unprojected and retain unsupported legacy behavior.
                assert!(game.loot_item_filter(&item_predicate("{h289:99b}"), &stack));
                assert_eq!(game.world.game_time, clock, "zero observation ticks");
            }
            assert!(
                failures.is_empty(),
                "actual mob-equipment custom_data mismatch: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod movement_predicate_tests {
    use serde_json::json;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn actual_registered_movement_predicate_uses_admitted_mob() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        game.block_ticks
            .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        assert!(game.world.loaded(4, 4));
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("movement-world");
        let pack = root.join("datapacks/movement");
        let folder = pack.join("data/motion/predicate");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"Movement fixture","min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
        let entity =
            |p| json!({"type":"minecraft:entity_properties","entity":"this","predicate":p});
        let movement =
            |p| entity(json!({"minecraft:entity_type":"minecraft:pig","minecraft:movement":p}));
        let high = movement(json!({"x":{"min":100}}));
        let zero = movement(json!({"x":{"min":0,"max":0}}));
        let probes = [
            ("high", high.clone(), false),
            ("zero", zero.clone(), true),
            ("empty", movement(json!({})), true),
            (
                "not_high",
                json!({"type":"minecraft:inverted","term":high}),
                true,
            ),
            (
                "not_zero",
                json!({"type":"minecraft:inverted","term":zero}),
                false,
            ),
        ];
        for (name, p, _) in &probes {
            std::fs::write(folder.join(format!("{name}.json")), p.to_string()).unwrap();
        }
        std::fs::write(
            folder.join("identity.json"),
            entity(json!({"minecraft:entity_type":"minecraft:pig"})).to_string(),
        )
        .unwrap();
        std::fs::write(
            folder.join("always.json"),
            r#"{"type":"minecraft:all_of","terms":[]}"#,
        )
        .unwrap();
        game.server.world_root = root;
        game.reload_datapacks(false, &[]).unwrap();
        assert!(game.run_command("scoreboard objectives add motion dummy"));
        assert!(game.run_command("execute if predicate motion:always"));
        let clock = game.world.game_time;
        let actor = "000002a1-0000-0000-0000-000000000001";
        assert!(game.run_command("summon pig 4.5 200 4.5 {UUID:[I;673,0,0,1],NoAI:1b,NoGravity:1b,Health:10f,Motion:[0d,0d,0d]}"));
        assert!(game.run_command(&format!("execute as {actor} if entity @s[type=pig,nbt={{UUID:[I;673,0,0,1],Health:10f,Pos:[4.5d,200d,4.5d],Motion:[0d,0d,0d],NoAI:1b,NoGravity:1b}}]")));
        assert!(game.run_command(&format!("execute as {actor} if predicate motion:identity")));
        let id = *game
            .persist
            .uuids
            .iter()
            .find(|(_, u)| **u == [673, 0, 0, 1])
            .unwrap()
            .0;
        let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
        assert_eq!(mob.velocity, Vec3::ZERO);
        assert_eq!(mob.fall_distance, 0.0);
        assert!(mob.no_ai && mob.no_gravity);
        assert!(mob.extra.mount.is_none());
        let view = game.entity_view_of(
            sunrise_gameplay_common::game::commands_common::Target::Mob(id),
        );
        assert_eq!(view.kind, "minecraft:pig");
        let mut failures = Vec::new();
        for (name, p, want) in probes {
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .predicates
                    .contains_key(&format!("motion:{name}"))
            );
            let direct = game.conditions_hold(&p, Some(&view), [4.5, 200.0, 4.5]);
            if direct != want {
                failures.push(format!("direct {name}: {direct} != {want}"));
            }
            assert!(game.run_command("scoreboard players set result motion 91"));
            assert!(game.run_command("scoreboard players set success motion 17"));
            game.run_command(&format!("execute store result score result motion store success score success motion as {actor} if predicate motion:{name}"));
            for key in ["result", "success"] {
                let value = game.score.board.score(key, "motion").unwrap().value;
                if value != i32::from(want) {
                    failures.push(format!("registered {name} {key}: {value}"));
                }
            }
        }
        assert_eq!(game.world.game_time, clock, "zero observation ticks");
        assert!(
            failures.is_empty(),
            "actual movement predicate mismatches: {failures:?}"
        );
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
