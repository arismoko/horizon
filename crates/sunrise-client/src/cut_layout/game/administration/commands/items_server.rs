#![cfg(test)]
use sunrise_gameplay::game::server_side::commands::items::Slot;

#[cfg(test)]
pub(crate) mod mob_equipment_command_tests {
    use crate::game::server_side::commands::items::mob_equipment_command_tests::with_game;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;

    #[test]
    #[ignore = "daily"]
    fn world_pack_enchantments_reach_actual_game_catalog() {
        use sunrise_core::random::LegacyRandom;
        use sunrise_items::enchant::EffectContext;
        with_game(|game| {
            let clock = game.world.game_time;
            game.server.world_root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("enchantment-catalog-world");
            let pack = game.server.world_root.join("datapacks/catalog");
            let definitions = pack.join("data/wpenchant/enchantment");
            std::fs::create_dir_all(&definitions).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"World enchantment catalog fixture"}}"#,
            )
            .unwrap();
            let definition = serde_json::json!({
                "description": "World catalog loaded", "supported_items": "minecraft:diamond_sword",
                "weight": 1, "max_level": 1,
                "min_cost": {"base": 1, "per_level_above_first": 0},
                "max_cost": {"base": 1, "per_level_above_first": 0},
                "anvil_cost": 1, "slots": ["mainhand"],
                "effects": {"minecraft:damage": [{"effect": {"type": "minecraft:add", "value": 2.0}}]}
            });
            std::fs::write(
                definitions.join("loaded.json"),
                serde_json::to_vec(&definition).unwrap(),
            )
            .unwrap();
            std::fs::write(
                game.server.world_root.join("datapacks.json"),
                r#"{"enabled":["file/catalog"]}"#,
            )
            .unwrap();
            let functions = pack.join("data/wpenchant/function");
            std::fs::create_dir_all(&functions).unwrap();
            std::fs::write(functions.join("ready.mcfunction"), "return 1\n").unwrap();
            let features = game.level.enabled_features.clone();
            let stack = game.reload_datapacks(false, &features).unwrap();
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .functions
                    .contains_key("wpenchant:ready")
            );
            let root = game.options.resources.clone();
            let prepared = game.prepare_world_reload(&stack, &root).unwrap();
            game.install_world_tables(prepared);
            game.run_native_advancement_grants();
            let raw_present = game
                .pack_files()
                .kind("enchantment")
                .any(|(id, files)| id == "wpenchant:loaded" && !files.is_empty());
            assert!(
                raw_present,
                "pack function admitted, but enchantment raw file absent; actual catalog entry present={}",
                game.stations()
                    .enchantments
                    .get("wpenchant:loaded")
                    .is_some()
            );
            assert!(std::sync::Arc::ptr_eq(
                &game.items().enchantments,
                &game.stations().enchantments
            ));
            assert!(
                game.stations()
                    .enchantments
                    .get("minecraft:mending")
                    .is_some()
            );
            assert!(
                game.stations()
                    .enchantments
                    .get("wpenchant:missing")
                    .is_none()
            );
            let entry = game
                .stations()
                .enchantments
                .get("wpenchant:loaded")
                .expect("the active Game catalog must adopt the loaded world-pack definition");
            assert_eq!(entry.description, "World catalog loaded");
            assert_eq!(entry.max_level, 1);
            assert!(game.stations().enchantments.can_enchant(
                game.items(),
                "wpenchant:loaded",
                "minecraft:diamond_sword"
            ));
            assert!(!game.stations().enchantments.can_enchant(
                game.items(),
                "wpenchant:loaded",
                "minecraft:stone"
            ));
            assert_eq!(
                game.stations().enchantments.modify_value_with(
                    "wpenchant:loaded",
                    1,
                    "minecraft:damage",
                    &EffectContext::default(),
                    &mut LegacyRandom::new(0),
                    1.0
                ),
                3.0
            );
            assert_eq!(game.world.game_time, clock);
        });
    }
}

#[cfg(test)]
pub(crate) mod registered_slot_source_read_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn registered_first_three_reads_match_inline_and_keep_the_chest() {
        let directory = tempfile::tempdir().unwrap();
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        let root = directory.path().join("registered-slot-read-world");
        assert!(!root.exists());
        let pack = root.join("datapacks/parity");
        std::fs::create_dir_all(pack.join("data/parity/slot_source")).unwrap();
        let meta = r#"{"pack":{"description":"parity slots","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#;
        let definition = r#"{"type":"minecraft:limit_slots","limit":3,"slot_source":{"type":"minecraft:slot_range","slots":"container.*"}}"#;
        std::fs::write(pack.join("pack.mcmeta"), meta).unwrap();
        std::fs::write(
            pack.join("data/parity/slot_source/first_three.json"),
            definition,
        )
        .unwrap();
        game.server.world_root = root;
        game.ensure_datapacks();
        assert_eq!(
            game.server
                .fns
                .0
                .data_resource("data/parity/slot_source/first_three.json")
                .unwrap()
                .read()
                .unwrap(),
            definition.as_bytes()
        );
        assert!(
            game.ui
                .registries
                .datapacks_enabled
                .iter()
                .any(|p| p == "file/parity")
        );
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 0 100 0 minecraft:chest{Items:[{Slot:0b,id:\"minecraft:stone\",count:5},{Slot:1b,id:\"minecraft:dirt\",count:2},{Slot:3b,id:\"minecraft:stone\",count:7},{Slot:4b,id:\"minecraft:shulker_box\",count:1,components:{\"minecraft:container\":[{slot:0,item:{id:\"minecraft:stone\",count:9}},{slot:2,item:{id:\"minecraft:dirt\",count:1}}]}}]}"));
        let before = game.block_entity_nbt([0, 100, 0]).unwrap();
        for (slot, item, count) in [
            (0, "minecraft:stone", 5),
            (1, "minecraft:dirt", 2),
            (3, "minecraft:stone", 7),
            (4, "minecraft:shulker_box", 1),
        ] {
            let actual = game.slot_get(&Slot::Block([0, 100, 0], slot)).unwrap();
            assert_eq!((&*actual.item, actual.count), (item, count));
        }
        assert!(game.run_command("scoreboard objectives add slotprobe dummy"));
        let clock = game.world.game_time;
        let inline =
            r#"{type:"limit_slots",limit:3,slot_source:{type:"slot_range",slots:"container.*"}}"#;
        let mut failures = Vec::new();
        for source in [inline, "parity:first_three"] {
            for (kind, suffix, count) in [
                ("items", " minecraft:stone", 5),
                ("items", " *", 7),
                ("slots", "", 3),
            ] {
                assert!(game.run_command("scoreboard players set #r slotprobe -99"));
                assert!(game.run_command("scoreboard players set #s slotprobe -99"));
                let command = format!(
                    "execute store result score #r slotprobe store success score #s slotprobe run execute if {kind} block 0 100 0 {source}{suffix}"
                );
                let ok = game.run_command(&command);
                let result = game.score.board.score("#r", "slotprobe").unwrap().value;
                let success = game.score.board.score("#s", "slotprobe").unwrap().value;
                println!("{command}: accepted={ok}, result={result}, success={success}");
                if (ok, result, success) != (true, count, 1) {
                    failures.push((command, ok, result, success));
                }
                assert_eq!(game.block_entity_nbt([0, 100, 0]).as_ref(), Some(&before));
                assert_eq!(game.world.game_time, clock);
            }
        }
        for name in ["#r", "#s"] {
            assert!(game.run_command(&format!("scoreboard players set {name} slotprobe -99")));
        }
        assert!(!game.run_command("execute store result score #r slotprobe store success score #s slotprobe run execute if slots block 0 100 0 parity:missing"));
        for name in ["#r", "#s"] {
            assert_eq!(
                game.score.board.score(name, "slotprobe").unwrap().value,
                -99
            );
        }
        assert_eq!(game.block_entity_nbt([0, 100, 0]).as_ref(), Some(&before));
        assert_eq!(game.world.game_time, clock);
        assert!(
            failures.is_empty(),
            "registered real-pack reads must match inline: {failures:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod registered_slot_source_preservation_tests {
    use sunrise_gameplay::game::server_side::commands::items::registered_slot_source_preservation_tests::count;

    use super::*;

    /// A command the player types runs as them (`CommandSourceStack.getEntity`),
    /// so `source:"this"` is their own slots without an `execute as`.
    #[test]
    #[ignore = "daily"]
    fn a_command_the_player_types_binds_this_to_them() {
        let mut resources = None;
        let directory = tempfile::tempdir().unwrap();
        let (mut game, _) = scene(
            &mut resources,
            directory.path().join("slot-source-player-this-world"),
        );
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 101.0, 0.5),
            )
            .unwrap(),
        );
        assert!(game.server.command_source.is_none());
        assert!(game.run_command(
            r#"item replace entity @s {type:"slot_range",source:"this",slots:"weapon.mainhand"} with minecraft:stone"#
        ));
        let hand = game.slot_get(&Slot::MainHand).unwrap();
        assert_eq!((&*hand.item, hand.count), ("minecraft:stone", 1));
    }

    #[test]
    #[ignore = "daily"]
    fn admitted_absent_executor_sources_fail_without_player_fallback() {
        let mut resources = None;
        let directory = tempfile::tempdir().unwrap();
        let (mut game, _) = scene(
            &mut resources,
            directory.path().join("slot-source-unsupported-world"),
        );
        let path = game
            .server
            .world_root
            .join("datapacks/parity/data/parity/slot_source/this.json");
        std::fs::write(
            path,
            r#"{"type":"minecraft:slot_range","source":"this","slots":"weapon.mainhand"}"#,
        )
        .unwrap();
        game.reload_datapacks(true, &[]).unwrap();
        let before = game.block_entity_nbt([0, 100, 0]).unwrap();
        let clock = game.world.game_time;
        count(&mut game, 3);
        let player_was_present = game.player.is_some();
        println!("absent executor: actual Player present={player_was_present}");
        let mut commands = vec![
            "item modify block 0 100 0 parity:this {type:\"minecraft:set_count\",count:1}"
                .to_owned(),
        ];
        for mode in ["replace", "fill", "override"] {
            commands.push(format!(
                "item {mode} block 0 100 0 parity:this with diamond"
            ));
            commands.push(format!(
                "item {mode} block 0 100 0 container.0 from block 0 100 0 parity:this"
            ));
        }
        for command in commands {
            let dispatcher = sunrise_command::vanilla::game_dispatcher();
            assert!(
                dispatcher
                    .parse(&command, &game.ui.registries)
                    .executable()
                    .is_ok(),
                "registered argument must be admitted: {command}"
            );
            assert!(
                !game.run_command(&command),
                "absent executor must not fall back to a Player: {command}"
            );
            assert_eq!(
                game.block_entity_nbt([0, 100, 0]).as_ref(),
                Some(&before),
                "{command}"
            );
            assert_eq!(game.player.is_some(), player_was_present);
            assert_eq!(game.world.game_time, clock);
        }
    }

    pub(crate) fn scene<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
        root: std::path::PathBuf,
    ) -> (sunrise_gameplay::game::ServerGame<'a>, std::path::PathBuf) {
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
        game.build_registries();
        game.set_allow_commands(true);
        assert!(!root.exists());
        let mut high = std::path::PathBuf::new();
        for (pack, limit) in [("a-low", 1), ("parity", 3)] {
            let dir = root.join("datapacks").join(pack);
            std::fs::create_dir_all(dir.join("data/parity/slot_source")).unwrap();
            std::fs::write(dir.join("pack.mcmeta"), r#"{"pack":{"description":"slot snapshot","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
            let path = dir.join("data/parity/slot_source/first_three.json");
            std::fs::write(&path, serde_json::json!({"type":"minecraft:limit_slots","limit":limit,"slot_source":{"type":"minecraft:slot_range","slots":"container.*"}}).to_string()).unwrap();
            high = path;
        }
        game.server.world_root = root;
        game.ensure_datapacks();
        assert!(game.run_command("setblock 0 100 0 chest{Items:[{Slot:0b,id:\"stone\",count:5},{Slot:1b,id:\"dirt\",count:2},{Slot:3b,id:\"stone\",count:7}]}"));
        assert!(game.run_command("scoreboard objectives add slotprobe dummy"));
        (game, high)
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
