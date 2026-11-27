#![cfg(test)]
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::commands_common::Target;

#[cfg(test)]
pub(crate) mod loot_mine_component_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay::game::server_side::commands::terrain_cmds::loot_mine_component_tests::slot;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn loot_mine_uses_the_actual_stand_or_server_executor_presence() {
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

        game.server.command_source = Some(sunrise_gameplay::game::commands::execute::Source {
            silent: false,
            ..game.server_source()
        });
        assert!(game.source_entity().is_none());
        assert!(game.run_command("execute unless entity @s"));
        assert!(game.run_command("setblock 12 200 4 chest strict"));
        assert!(game.run_command("setblock 2 200 4 snow[layers=1] strict"));
        assert!(game.run_command("setblock 4 200 4 dirt strict"));
        assert!(game.run_command("item replace block 12 200 4 container.26 with diamond 7"));
        assert!(game.run_command("item replace block 12 200 4 container.2 with diamond_shovel"));
        assert_eq!(&*slot(&game, 2).item, "minecraft:diamond_shovel");
        assert!(game.run_command("execute if items block 12 200 4 container.2 #minecraft:shovels"));
        assert!(game.run_command(
            "summon armor_stand 8.5 200 8.5 {UUID:[I;60,0,0,1],Health:20f,NoGravity:1b}"
        ));
        let stand = "0000003c-0000-0000-0000-000000000001";
        assert!(game.run_command(&format!("execute as {stand} if entity @s[type=armor_stand,nbt={{UUID:[I;60,0,0,1],Health:20f,NoGravity:1b,Pos:[8.5d,200d,8.5d]}}]")));
        let id = game
            .armor_stands
            .iter()
            .find(|s| game.persist.uuids.get(&s.id) == Some(&[60, 0, 0, 1]))
            .unwrap()
            .id;
        let before_stand = game.command_target_nbt(Target::Stand(id)).unwrap();
        assert_eq!(
            before_stand.get("UUID"),
            Some(&Tag::IntArray(vec![60, 0, 0, 1]))
        );
        let clock = game.world.game_time;
        // Unconditional dirt proves both sources really execute the same path.
        for (as_stand, x, expected) in [
            (false, 4, "minecraft:dirt"),
            (true, 4, "minecraft:dirt"),
            (false, 2, "minecraft:emerald"),
            (true, 2, "minecraft:snowball"),
        ] {
            assert!(game.source_entity().is_none());
            assert!(game.run_command("item replace block 12 200 4 container.0 with emerald 4"));
            let before = slot(&game, 0);
            let state = game.world.block(x, 200, 4);
            let prefix = if as_stand {
                format!("execute as {stand} run ")
            } else {
                String::new()
            };
            assert!(game.run_command(&format!(
                "{prefix}loot replace block 12 200 4 container.0 mine {x} 200 4 diamond_shovel"
            )));
            let actual = slot(&game, 0);
            assert_eq!(
                &*actual.item, expected,
                "executor stand={as_stand}, block x={x}"
            );
            if !as_stand && x == 2 {
                assert_eq!(actual, before);
            } else {
                assert_eq!(actual.count, 1);
            }
            assert_eq!(
                (&*slot(&game, 26).item, slot(&game, 26).count),
                ("minecraft:diamond", 7)
            );
            assert_eq!(&*slot(&game, 2).item, "minecraft:diamond_shovel");
            assert_eq!(game.world.block(x, 200, 4), state);
            assert_eq!(
                game.command_target_nbt(Target::Stand(id)).unwrap(),
                before_stand
            );
            assert_eq!(game.world.game_time, clock);
        }
    }
}

#[cfg(test)]
pub(crate) mod loot_mine_location_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_gameplay::game::commands::terrain_cmds::loot_mine_location_tests::TABLE;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};

    #[test]
    #[ignore = "daily"]
    fn loot_mine_location_reads_actual_offset_block_state() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("h079-game"),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();

            game.world
                .chunks
                .insert((-1, -1), Arc::new(Chunk::empty(-1, -1)));
            game.world
                .chunks
                .insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
            game.world
                .chunks
                .insert((0, -1), Arc::new(Chunk::empty(0, -1)));
            for (x, z) in [(-1, -1), (-1, 0), (0, -1)] {
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(x, z))
                    .unwrap();
            }
            game.server.world_root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                    .join("h079-world");
            let pack = game.server.world_root.join("datapacks/h079");
            std::fs::create_dir_all(pack.join("data/minecraft/loot_table/blocks")).unwrap();
            std::fs::create_dir_all(pack.join("data/h079/function")).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"H079 fixture"}}"#,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/minecraft/loot_table/blocks/stone.json"),
                TABLE,
            )
            .unwrap();
            std::fs::write(
                pack.join("data/h079/function/admit.mcfunction"),
                "scoreboard players set pack h079 42\n",
            )
            .unwrap();
            std::fs::write(
                game.server.world_root.join("datapacks.json"),
                r#"{"enabled":["vanilla","file/h079"],"disabled":[]}"#,
            )
            .unwrap();
            let stack = game.reload_datapacks(false, &[]).unwrap();
            game.server.world_tables.0 = game
                .stage_world_tables(&stack, false, &game.options.resources.clone())
                .unwrap()
                .tables;
            assert!(game.server.world_tables.0.is_some());
            game.ensure_datapacks();
            assert!(game.run_command("scoreboard objectives add h079 dummy"));
            game.run_command("function h079:admit");
            assert!(
                game.run_command("execute if score pack h079 matches 42"),
                "real pack function admission"
            );
            assert!(game.run_command("forceload add -16 -16 15 15"));
            assert!(game.run_command("fill -16 199 -16 15 199 15 stone"));
            let clock = game.world.game_time;
            let mut differences = Vec::new();
            for (label, x, z, counterpart, expected) in [
                ("correct", 4, 4, Some("oak_log[axis=x]"), 1),
                ("wrong-state", 8, 4, Some("oak_log[axis=z]"), 0),
                ("absent", 12, 4, None, 0),
                ("negative-center", -4, -4, Some("oak_log[axis=x]"), 1),
            ] {
                assert!(game.run_command(&format!("setblock {x} 200 {z} stone strict")));
                if let Some(other) = counterpart {
                    assert!(
                        game.run_command(&format!("setblock {} 200 {z} {other} strict", x + 1))
                    );
                }
                assert!(game.run_command(&format!(
                    "execute if block {} 200 {z} {}",
                    x + 1,
                    counterpart.unwrap_or("air")
                )));
                let before = (game.world.block(x, 200, z), game.world.block(x + 1, 200, z));
                assert!(game.run_command(&format!("setblock {x} 200 10 chest strict")));
                let Some(BlockEntity::Chest(initial)) =
                    game.world.block_entities.get(&(x, 200, 10))
                else {
                    panic!("real output chest")
                };
                assert!(initial.iter().all(Option::is_none));
                let prefix = if x < 0 {
                    "execute positioned -8.25 200.125 -8.75 run "
                } else {
                    ""
                };
                assert!(game.run_command(&format!(
                    "{prefix}loot insert {x} 200 10 mine {x} 200 {z} stick"
                )));
                let Some(BlockEntity::Chest(output)) = game.world.block_entities.get(&(x, 200, 10))
                else {
                    panic!("actual output")
                };
                assert!(
                    output
                        .iter()
                        .flatten()
                        .any(|s| s.item.as_ref() == "minecraft:emerald" && s.count == 2),
                    "pack override and unconditional roll are real"
                );
                let diamonds: u32 = output
                    .iter()
                    .flatten()
                    .filter(|s| s.item.as_ref() == "minecraft:diamond")
                    .map(|s| s.count)
                    .sum();
                if diamonds != expected {
                    differences.push((label, diamonds, expected));
                }
                assert_eq!(
                    (game.world.block(x, 200, z), game.world.block(x + 1, 200, z)),
                    before
                );
            }
            assert_eq!(game.world.game_time, clock);
            assert!(
                differences.is_empty(),
                "actual offset block predicate differences: {differences:?}"
            );
        });
    }
}

#[cfg(test)]
#[path = "loot_slot_tests_server.rs"]
pub(crate) mod loot_slot_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
