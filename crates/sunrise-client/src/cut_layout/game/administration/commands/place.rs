#![cfg(test)]
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
#[cfg(test)]
pub(crate) mod live_dungeon_spawner_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};
    use sunrise_sim::{
        mobs::spawner::{SpawnDataExt, SpawnerExt},
        motion::PlayerMotion,
    };
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
                .join(format!("h173-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        sunrise_gameplay_common::game::growth_features::install_live_features(
            &loaded.resources,
            &loaded.pack,
            &options,
            &Default::default(),
        );
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

        for cx in -1..=1 {
            for cz in -1..=1 {
                game.world
                    .chunks
                    .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(cx, cz))
                    .unwrap();
            }
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("forceload add -16 -16 31 31"));
        assert!(game.run_command("tick freeze"));
        assert!(game.run_command("fill 0 198 0 31 199 31 stone"));
        test(&mut game);
    }

    #[test]
    #[ignore = "daily"]
    fn a_registered_live_dungeon_keeps_its_real_selected_spawner() {
        with_game(|game| {
            let clock = game.world.game_time;
            assert!(
                game.res
                    .behaviours()
                    .live_features()
                    .unwrap()
                    .configured("minecraft:monster_room")
                    .is_some()
            );
            for cmd in [
                "fill 3 199 3 13 204 13 barrier strict",
                "fill 3 199 3 13 204 13 stone strict",
                "fill 6 200 6 10 203 10 air strict",
                "fill 11 200 8 13 201 8 air strict",
                "setblock 24 200 24 spawner{SpawnData:{entity:{id:\"minecraft:zombie\"}},Delay:20s} strict",
            ] {
                assert!(game.run_command(cmd), "{cmd}");
            }
            assert!(game.run_command("execute if block 8 200 8 air"));
            assert!(game.world.block_entities.get(&(8, 200, 8)).is_none());
            let Some(BlockEntity::Spawner(direct)) = game.world.block_entities.get(&(24, 200, 24))
            else {
                panic!("direct spawner positive")
            };
            assert_eq!(
                direct.next_data().and_then(|d| d.id()).as_deref(),
                Some("minecraft:zombie")
            );
            assert!(game.world.chunks.contains_key(&(-1, -1)));
            assert!(game.world.chunks.contains_key(&(1, 1)));
            game.world.random.set_seed(173);
            assert!(
                game.run_command("place feature minecraft:monster_room 8 200 8"),
                "actual live feature admission"
            );
            assert!(game.run_command("execute if block 8 200 8 spawner"));
            let mut after = game.world.random.clone();
            let continuation: Vec<i64> = (0..4).map(|_| after.next_long()).collect();
            println!(
                "h173-continuation {}",
                serde_json::to_string(&continuation).unwrap()
            );
            let Some(BlockEntity::Spawner(spawner)) = game.world.block_entities.get(&(8, 200, 8))
            else {
                panic!("actual generated spawner is missing from live collection")
            };
            let chosen = spawner
                .next_data()
                .and_then(|d| d.id())
                .expect("actual selected entity");
            assert!(
                ["minecraft:skeleton", "minecraft:zombie", "minecraft:spider"]
                    .contains(&chosen.as_str())
            );
            assert!(spawner.potentials.is_empty());
            let tag = game.block_entity_nbt([8, 200, 8]).unwrap();
            for (key, value) in [
                ("id", Tag::String("minecraft:mob_spawner".into())),
                ("x", Tag::Int(8)),
                ("y", Tag::Int(200)),
                ("z", Tag::Int(8)),
            ] {
                assert_eq!(tag.get(key), Some(&value));
            }
            assert_eq!(
                tag.get("SpawnData")
                    .and_then(|t| t.get("entity"))
                    .and_then(|t| t.get("id")),
                Some(&Tag::String(chosen.as_str().into()))
            );
            assert!(game.run_command("execute if data block 8 200 8 SpawnData.entity.id"));
            assert!(game.run_command("execute if data block 24 200 24 {Delay:20s,SpawnData:{entity:{id:\"minecraft:zombie\"}}}"));
            assert_eq!(game.world.game_time, clock);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn creation_boundary_preserves_existing_entries_and_rejects_other_blocks() {
        with_game(|game| {
            use sunrise_world::level::LevelAccess;
            let request=match sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:mob_spawner",SpawnData:{entity:{id:"minecraft:skeleton"}},SpawnPotentials:[]}"#).unwrap().0 {Tag::Compound(c)=>c,_=>unreachable!()};
            assert!(game.run_command(r#"setblock 24 200 24 spawner{Delay:77s,MaxNearbyEntities:9s,SpawnData:{entity:{id:"minecraft:zombie",UUID:[I;173,4,5,6],Health:7f}},SpawnPotentials:[]} strict"#));
            if let Some(BlockEntity::Spawner(s)) = game.world.block_entities.get_mut(&(24, 200, 24))
            {
                s.spin = 12.5;
                s.o_spin = 6.25;
                s.client_delay = 33;
            }
            let before = game.block_entity_nbt([24, 200, 24]).unwrap();
            assert_eq!(
                before
                    .get("SpawnData")
                    .and_then(|v| v.get("entity"))
                    .and_then(|v| v.get("UUID")),
                Some(&Tag::IntArray(vec![173, 4, 5, 6]))
            );
            let model = format!("{:?}", game.world.block_entities.get(&(24, 200, 24)));
            let random = game.world.random.clone().next_long();
            game.with_level(|_, level, _| {
                level.create_feature_spawner(BlockPos::new(24, 200, 24), request.clone())
            });
            assert_eq!(game.block_entity_nbt([24, 200, 24]).unwrap(), before);
            assert_eq!(
                format!("{:?}", game.world.block_entities.get(&(24, 200, 24))),
                model
            );
            assert_eq!(game.world.random.clone().next_long(), random);
            assert!(game.run_command("setblock 26 200 24 spawner strict"));
            let mut wrong = BlockEntity::for_block("minecraft:chest").unwrap();
            wrong.slots_mut()[0] =
                Some(sunrise_items::items::ItemStack::new("minecraft:diamond", 3));
            game.world.block_entities.insert((26, 200, 24), wrong);
            let before = format!("{:?}", game.world.block_entities.get(&(26, 200, 24)));
            game.with_level(|_, level, _| {
                level.create_feature_spawner(BlockPos::new(26, 200, 24), request.clone())
            });
            assert_eq!(
                format!("{:?}", game.world.block_entities.get(&(26, 200, 24))),
                before
            );
            assert!(game.run_command("setblock 28 200 24 stone strict"));
            let revision = game.world.revision;
            let count = game.world.block_entities.len();
            game.with_level(|_, level, _| {
                level.create_feature_spawner(BlockPos::new(28, 200, 24), request.clone())
            });
            assert!(game.world.block_entities.get(&(28, 200, 24)).is_none());
            assert_eq!(game.world.revision, revision);
            assert_eq!(game.world.block_entities.len(), count);
        });
    }
}
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod source_tests {
    #[cfg(feature = "client")]
    use std::{io::Write, path::PathBuf};
    #[cfg(feature = "client")]
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    #[cfg(feature = "client")]
    use sunrise_core::nbt::{self, Limits, Tag};
    #[cfg(feature = "client")]
    use sunrise_gameplay::game::server_side::commands::place::read_template_data;
    #[cfg(feature = "client")]
    use sunrise_world::compression::Compression;

    #[cfg(feature = "client")]
    #[test]
    fn template_tags_read_archives_below_saved_world_structures() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-command-template-source-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(root);
        let bytes = |value: &str| {
            let named = nbt::NamedTag {
                name: "".into(),
                tag: Tag::compound([("marker", Tag::String(value.into()))]),
            };
            Compression::Gzip
                .encode(&nbt::write_named(&named, Limits::templates()).unwrap())
                .unwrap()
        };
        let prepared = scratch.0.join("prepared.nbt");
        let saved = scratch.0.join("saved/probe.nbt");
        let jar = scratch.0.join("templates.jar");
        std::fs::write(&prepared, bytes("base")).unwrap();
        {
            let mut archive = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
            archive
                .start_file(
                    "data/example/structure/probe.nbt",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            archive.write_all(&bytes("archive")).unwrap();
            archive.finish().unwrap();
        }
        let source = ResourceSource::archive(&jar).unwrap();
        let resource = Resource::file(source, "data/example/structure/probe.nbt".into());
        let marker = |resource| {
            read_template_data(&saved, &prepared, resource)
                .unwrap()
                .get("marker")
                .cloned()
                .unwrap()
        };
        assert_eq!(
            marker(Some(resource.clone())),
            Tag::String("archive".into())
        );
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, bytes("saved")).unwrap();
        assert_eq!(marker(Some(resource.clone())), Tag::String("saved".into()));
        std::fs::write(&saved, b"invalid gzip").unwrap();
        assert_eq!(
            marker(Some(resource.clone())),
            Tag::String("archive".into())
        );
        std::fs::remove_file(&saved).unwrap();
        assert!(
            read_template_data(
                &saved,
                &prepared,
                Some(Resource::bytes(b"invalid gzip".to_vec()))
            )
            .is_none()
        );
        assert_eq!(marker(None), Tag::String("base".into()));
        std::fs::remove_file(&jar).unwrap();
        assert_eq!(marker(Some(resource)), Tag::String("archive".into()));
        assert!(!scratch.0.join("data").exists());
    }
}
