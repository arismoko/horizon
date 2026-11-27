#![cfg(test)]
#[cfg(test)]
pub(crate) mod enchantment_snapshot_tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    use {
        crate::game::resource_load::load_resources, sunrise_client_render::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn synchronized_definitions_refresh_existing_enchantment_views() {
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

        let dir = std::env::temp_dir().join(format!("sunrise-h024-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/model");
        let tags = pack.join("data/h024/tags/enchantment");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture"}}"#,
        )
        .unwrap();
        std::fs::write(tags.join("empty.json"), r#"{"values":[]}"#).unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/model"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = dir;
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let root = game.options.resources.clone();
        game.server.world_tables.0 = game
            .stage_world_tables(&stack, false, &root)
            .unwrap()
            .tables;
        assert!(
            game.server.world_tables.0.is_some(),
            "the real active table container must be available"
        );
        let base = game.stations().enchantments.clone();
        assert!(base.get("minecraft:sharpness").is_some());
        // Same definition independently admitted by fresh Java. This is controlled
        // synchronized model input, NOT a world-pack or packet-delivery claim.
        let definition = serde_json::json!({"description":"H024 audit","supported_items":"minecraft:diamond_sword",
            "weight":1,"max_level":1,"min_cost":{"base":1,"per_level_above_first":0},
            "max_cost":{"base":1,"per_level_above_first":0},"anvil_cost":1,"slots":["mainhand"],"effects":{}});
        let synced = std::sync::Arc::new(
            base.with_entries([("h024:audit", &definition)])
                .unwrap()
                .unwrap(),
        );
        game.server
            .world_tables
            .0
            .as_mut()
            .unwrap()
            .stations
            .enchantments = synced.clone();
        std::sync::Arc::make_mut(&mut game.server.world_tables.0.as_mut().unwrap().items)
            .enchantments = synced;
        assert!(
            game.active_enchantment_definitions()
                .get("h024:audit")
                .is_some(),
            "actual authority adopts the definition first"
        );
        let tags = vec!["h024:retained_tag".to_owned()];
        game.ui
            .registries
            .by_registry
            .entry("minecraft:enchantment".into())
            .or_default()
            .tags = tags.clone();
        let item_ids = game.ui.registries.ids("minecraft:item").to_vec();
        let effects = game.ui.registries.effects.clone();
        game.sync_function_registries();
        for phase in [0, 1] {
            if phase == 1 {
                game.build_registries();
            }
            let reg = &game.ui.registries;
            assert!(
                reg.ids("minecraft:enchantment")
                    .contains(&"h024:audit".to_owned()),
                "active custom ID reaches actual Game sync/build snapshot"
            );
            assert!(
                reg.ids("minecraft:enchantment")
                    .contains(&"minecraft:sharpness".to_owned())
            );
            assert!(
                !reg.ids("minecraft:enchantment")
                    .contains(&"h024:missing".to_owned())
            );
            assert_eq!(
                reg.ids("minecraft:enchantment"),
                reg.enchantments.as_slice()
            );
            assert_eq!(reg.by_registry["minecraft:enchantment"].tags, tags);
            assert_eq!(reg.ids("minecraft:item"), item_ids);
            assert_eq!(reg.effects, effects);
        }
        game.server
            .world_tables
            .0
            .as_mut()
            .unwrap()
            .stations
            .enchantments = base.clone();
        std::sync::Arc::make_mut(&mut game.server.world_tables.0.as_mut().unwrap().items)
            .enchantments = base;
        assert!(
            game.active_enchantment_definitions()
                .get("h024:audit")
                .is_none()
        );
        game.sync_function_registries();
        assert!(
            !game
                .ui
                .registries
                .ids("minecraft:enchantment")
                .contains(&"h024:audit".to_owned()),
            "refresh replaces rather than accumulating stale IDs"
        );
        assert_eq!(
            game.ui.registries.ids("minecraft:enchantment"),
            game.ui.registries.enchantments.as_slice()
        );
        assert_eq!(
            game.ui.registries.by_registry["minecraft:enchantment"].tags,
            tags
        );
    }
}

#[cfg(test)]
pub(crate) mod damage_type_registry_tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use {
        crate::game::resource_load::load_resources, sunrise_client_render::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    /// A world pack's damage type is a `/damage` type once the world opens,
    /// beside vanilla's, and the registry keeps vanilla's tags.
    #[test]
    #[ignore = "daily"]
    fn a_world_packs_damage_type_parses_in_damage() {
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

        let dir = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join("damage-type-ids");
        let pack = dir.join("datapacks/model");
        let types = pack.join("data/example/damage_type");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"test","pack_format":121}}"#,
        )
        .unwrap();
        std::fs::write(
            types.join("test.json"),
            r#"{"message_id":"example.test","scaling":"never","exhaustion":0.0}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("datapacks.json"),
            r#"{"enabled":["file/model"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = dir;
        let stack = game.reload_datapacks(false, &[]).unwrap();
        let staged = game.stage_world_tables(&stack, false, &root).unwrap();
        game.server.world_tables.0 = staged.tables;
        game.health_resources = staged.health;
        game.level.allow_commands = Some(true);
        game.build_registries();

        let reg = &game.ui.registries;
        assert!(
            reg.damage_types.contains(&"example:test".to_owned()),
            "the damage catalog holds the pack's type"
        );
        let parse = |input: &str| {
            sunrise_command::vanilla::dispatcher()
                .parse(input, reg)
                .executable()
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        assert_eq!(parse("damage @s 1 example:test"), Ok(()));
        assert_eq!(parse("damage @s 1 minecraft:magic"), Ok(()));
        assert!(parse("damage @s 1 example:missing").is_err());
        assert!(
            reg.tags("minecraft:damage_type")
                .contains(&"minecraft:is_fire".to_owned()),
            "vanilla's damage type tags stay"
        );
    }
}

#[cfg(test)]
#[path = "tests/end_clock_tests_server.rs"]
pub(crate) mod end_clock_tests;
