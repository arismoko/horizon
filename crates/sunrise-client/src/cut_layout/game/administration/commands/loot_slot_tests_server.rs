use crate::game::server_side::commands::terrain_cmds::loot_slot_tests::with_game;
use sunrise_gameplay::game::commands::items::Slot;

#[test]
#[ignore = "daily"]
fn loot_kill_matches_the_world_pack_entity_type_tag() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    with_game(|game| {
        let world = tempfile::tempdir().unwrap();
        let pack = world.path().join("datapacks/loot/data");
        std::fs::create_dir_all(pack.join("snapshot/tags/entity_type")).unwrap();
        std::fs::create_dir_all(pack.join("minecraft/loot_table/entities")).unwrap();
        std::fs::write(
            pack.parent().unwrap().join("pack.mcmeta"),
            r#"{"pack":{"description":"loot snapshot fixture"}}"#,
        )
        .unwrap();
        std::fs::write(
            pack.join("snapshot/tags/entity_type/target.json"),
            r#"{"values":["minecraft:sheep"]}"#,
        )
        .unwrap();
        std::fs::write(
            pack.join("minecraft/loot_table/entities/sheep.json"),
            r##"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","condition":{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"#snapshot:target"}}}]}]}"##,
        ).unwrap();
        std::fs::write(
            world.path().join("datapacks.json"),
            r#"{"enabled":["file/loot"],"disabled":[]}"#,
        )
        .unwrap();
        game.server.world_root = world.path().to_path_buf();
        game.reload_datapacks(true, &[]).unwrap();
        assert!(game.run_command("setblock 4 200 4 chest"));
        assert!(game.run_command("summon minecraft:sheep 8 200 8 {NoAI:1b}"));
        assert!(game.run_command(
            "loot replace block 4 200 4 container.0 1 kill @e[type=minecraft:sheep,limit=1]"
        ));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|stack| (stack.item.to_string(), stack.count)),
            Some(("minecraft:diamond".into(), 1)),
        );
    });
}

#[test]
#[ignore = "daily"]
fn server_loot_without_world_tables_owns_its_resource_root() {
    with_game(|game| {
        for count in [2, 3] {
            let root = tempfile::tempdir().unwrap();
            let tables = root.path().join("data/snapshot/loot_table");
            std::fs::create_dir_all(&tables).unwrap();
            std::fs::write(root.path().join("items.json"), "{}").unwrap();
            std::fs::write(
                tables.join("reward.json"),
                format!(r#"{{"pools":[{{"rolls":{count},"entries":[{{"type":"minecraft:item","name":"minecraft:diamond"}}]}}]}}"#),
            ).unwrap();
            game.options.resources = root.path().to_path_buf();
            game.load_server_loot();
            let loot = game.server.server_loot.0.get().unwrap();
            let drops = loot.roll_split(
                "snapshot:reward",
                &mut sunrise_core::random::LegacyRandom::new(0),
            );
            assert_eq!(drops.iter().map(|stack| stack.count).sum::<i32>(), count);
            assert!(drops.iter().all(|stack| stack.item == "minecraft:diamond"));
        }
    });
}
