#![cfg(test)]
use crate::settings::Settings;

use sunrise_core::position::ChunkPos;

use crate::game::load_resources;

#[cfg(test)]
pub(crate) mod loot_mine_component_tests {

    use sunrise_gameplay_common::game::server_side::commands::terrain_cmds::loot_mine_component_tests::decorations;

    use sunrise_gameplay::game::server_side::commands::terrain_cmds::loot_mine_component_tests::slot;

    use super::*;

    use crate::game::{Options, Progress, Settings, load_resources};

    use std::sync::Arc;

    use sunrise_core::nbt::{Compound, Tag};

    use sunrise_level::chunk::Chunk;

    #[test]
    #[ignore = "daily"]
    fn loot_mine_copies_actual_empty_and_right_pot_components_into_the_chest() {
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
        assert!(game.run_command(
            "setblock 12 200 4 chest[facing=north,type=single,waterlogged=false] strict"
        ));
        assert!(
            game.run_command("item replace block 12 200 4 container.26 with minecraft:diamond 7")
        );
        assert!(game.run_command("item replace block 12 200 4 container.2 with minecraft:decorated_pot[minecraft:pot_decorations={}] 1"));
        let empty = Tag::Compound(Compound::new());
        assert_eq!(
            (&*slot(&game, 26).item, slot(&game, 26).count),
            ("minecraft:diamond", 7)
        );
        assert_eq!(
            decorations(&slot(&game, 2)),
            Some(empty.clone()),
            "direct-item parser/encoder positive control"
        );
        assert!(
            slot(&game, 2)
                .extra
                .as_ref()
                .unwrap()
                .pot_decorations
                .is_some()
        );
        let start = game.world.game_time;
        let right = "minecraft:angler_pottery_sherd";
        let mut failures = Vec::new();
        for (x, nbt, decorated) in [
            (4, "", false),
            (
                7,
                r#"{sherds:{right:{id:"minecraft:angler_pottery_sherd"}}}"#,
                true,
            ),
        ] {
            assert!(game.run_command(&format!(
                "setblock {x} 200 4 decorated_pot[facing=north,cracked=false,waterlogged=false]{nbt} strict"
            )));
            let pos = [x, 200, 4];
            let state = game.world.block(x, 200, 4);
            let actual = game.pack.assets.registry.state(state).unwrap();
            assert_eq!(&*actual.name, "minecraft:decorated_pot");
            assert_eq!(
                actual.properties.get("cracked").map(String::as_str),
                Some("false")
            );
            assert_eq!(
                game.world.block(x, 199, 4),
                game.pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap()
            );
            let before = game.pot_data(pos).expect("actual source block entity read");
            assert_eq!(
                before.get("id"),
                Some(&Tag::String("minecraft:decorated_pot".into()))
            );
            for (key, value) in [("x", x), ("y", 200), ("z", 4)] {
                assert_eq!(before.get(key), Some(&Tag::Int(value)));
            }
            if decorated {
                assert_eq!(
                    before
                        .get("sherds")
                        .and_then(|t| t.get("right"))
                        .and_then(|t| t.get("id")),
                    Some(&Tag::String(right.into())),
                    "actual admitted source sherd"
                );
            } else {
                assert!(before.get("sherds").is_none());
            }
            assert!(before.get("item").is_none() && before.get("LootTable").is_none());
            assert!(
                game.run_command("item replace block 12 200 4 container.0 with minecraft:stone 3")
            );
            let initial = slot(&game, 0);
            assert_eq!((&*initial.item, initial.count), ("minecraft:stone", 3));
            assert!(game.run_command(&format!(
                "loot replace block 12 200 4 container.0 1 mine {x} 200 4"
            )));
            let out = slot(&game, 0);
            assert_eq!(
                (&*out.item, out.count),
                ("minecraft:decorated_pot", 1),
                "inventory is the count authority"
            );
            let expected = if decorated {
                Tag::compound([("right", Tag::compound([("id", Tag::String(right.into()))]))])
            } else {
                empty.clone()
            };
            if decorations(&out) != Some(expected) {
                failures.push(format!(
                    "source {x}: actual saved output pot_decorations missing/wrong"
                ));
            }
            let modeled = out.extra.as_ref().and_then(|d| d.pot_decorations.as_ref());
            if modeled.is_none_or(|sides| {
                sides.iter().enumerate().any(|(i, item)| {
                    if decorated && i == 2 {
                        item.as_ref()
                            .is_none_or(|s| &*s.item != right || s.count != 1)
                    } else {
                        item.is_some()
                    }
                })
            }) {
                failures.push(format!(
                    "source {x}: actual output component state missing/wrong"
                ));
            }
            assert_eq!(
                game.pot_data(pos),
                Some(before),
                "mine leaves the actual source unchanged"
            );
            assert_eq!(game.world.block(x, 200, 4), state);
            assert_eq!(
                (&*slot(&game, 26).item, slot(&game, 26).count),
                ("minecraft:diamond", 7)
            );
            assert_eq!(decorations(&slot(&game, 2)), Some(empty.clone()));
            assert_eq!(game.world.game_time, start);
        }
        assert!(game.run_command("setblock 10 200 4 dirt strict"));
        assert!(game.run_command("loot replace block 12 200 4 container.0 1 mine 10 200 4"));
        let plain = slot(&game, 0);
        assert_eq!((&*plain.item, plain.count), ("minecraft:dirt", 1));
        assert!(decorations(&plain).is_none());
        assert_eq!(game.world.game_time, start);
        assert!(
            failures.is_empty(),
            "actual loot-mine component-copy failures: {failures:#?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod loot_position_tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use {
        crate::game::{Settings, load_resources},
        sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn actual_loot_replacement_retains_the_emitted_zero_slot() {
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
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 4 200 4 chest"));
        let slot = |game: &mut sunrise_gameplay::game::ServerGame<'_>, i| {
            game.slot_get(
                &sunrise_gameplay::game::server_side::commands::items::Slot::Block([4, 200, 4], i),
            )
        };
        let reset = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            assert!(game.run_command("item replace block 4 200 4 container.0 with diamond 3"));
            assert!(game.run_command("item replace block 4 200 4 container.1 with emerald 4"));
            assert!(game.run_command("item replace block 4 200 4 container.2 with gold_ingot 5"));
            assert_eq!(
                slot(game, 0).map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 3))
            );
            assert_eq!(
                slot(game, 1).map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:emerald".into(), 4))
            );
        };
        let table = |counts: &[i32]| {
            serde_json::json!({"pools":counts.iter().map(|count| serde_json::json!({
            "rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot",
                "modifier":{"type":"minecraft:set_count","count":count}}]
        })).collect::<Vec<_>>()})
            .to_string()
        };
        let time = game.world.game_time;
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 loot {}",
            table(&[1])
        )));
        assert_eq!(
            slot(&mut game, 0).map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:iron_ingot".into(), 1))
        );
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 loot {}",
            table(&[0, 1])
        )));
        assert!(
            slot(&mut game, 0).is_none(),
            "the emitted zero occupies and clears slot0"
        );
        assert_eq!(
            slot(&mut game, 1).map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:iron_ingot".into(), 1))
        );
        assert_eq!(slot(&mut game, 2).unwrap().count, 5);
        reset(&mut game);
        assert!(game.run_command(&format!(
            "loot replace block 4 200 4 container.0 2 loot {}",
            table(&[])
        )));
        assert!(slot(&mut game, 0).is_none());
        assert!(slot(&mut game, 1).is_none());
        assert_eq!(slot(&mut game, 2).unwrap().count, 5);
        assert_eq!(game.world.game_time, time);
    }
}

#[cfg(test)]
pub(crate) mod loot_kill_target_tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity};
    use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};
    #[test]
    #[ignore = "daily"]
    fn loot_kill_uses_actual_sheep_state_without_killing_the_target() {
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
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("h036-game"),
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
        // A real world open publishes its owned loot evaluator before commands run.
        game.load_server_loot();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
        let clock = game.world.game_time;
        let mut differences = Vec::new();
        for (n, color, sheared, wool) in [
            (1, 0, true, "minecraft:white_wool"),
            (2, 14, true, "minecraft:red_wool"),
            (3, 0, false, "minecraft:white_wool"),
            (4, 14, false, "minecraft:red_wool"),
        ] {
            let uuid = format!("00000024-0000-0000-0000-{n:012x}");
            assert!(game.run_command(&format!(
                "summon sheep 4.5 200 4.5 {{UUID:[I;36,0,0,{n}],Health:8f,NoAI:1b,NoGravity:1b,Age:0,Color:{color}b,Sheared:{}b,Motion:[0d,0d,0d]}}", i32::from(sheared)
            )));
            let mob = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([36, 0, 0, n]))
                .unwrap();
            assert_eq!(mob.kind.id, "minecraft:sheep");
            assert_eq!(mob.health, 8.0);
            assert!(mob.no_ai && mob.no_gravity);
            assert_eq!(mob.extra.color, color);
            assert_eq!(mob.extra.sheared, sheared);
            let before = (mob.id, mob.position, mob.velocity, mob.health);
            let x = 2 + n * 3;
            assert!(game.run_command(&format!(
                "setblock {x} 200 8 chest[facing=north,type=single,waterlogged=false] strict"
            )));
            let Some(BlockEntity::Chest(initial)) = game.world.block_entities.get(&(x, 200, 8))
            else {
                panic!("real chest required")
            };
            assert!(initial.iter().all(Option::is_none));
            assert!(game.run_command(&format!("loot insert {x} 200 8 kill {uuid}")));
            let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(x, 200, 8))
            else {
                panic!("real output chest required")
            };
            let output: Vec<_> = items.iter().flatten().collect();
            assert!(
                output
                    .iter()
                    .any(|s| s.item.as_ref() == "minecraft:mutton" && (1..=2).contains(&s.count)),
                "nonempty meat proves the command actually rolled"
            );
            let actual: u32 = output
                .iter()
                .filter(|s| s.item.as_ref() == wool)
                .map(|s| s.count)
                .sum();
            if actual != u32::from(!sheared) {
                differences.push((n, sheared, wool, actual));
            }
            let mob = game.mobs.iter().find(|m| m.id == before.0).unwrap();
            assert_eq!((mob.id, mob.position, mob.velocity, mob.health), before);
            assert_eq!(game.persist.uuid_of(mob.id), Some([36, 0, 0, n]));
            assert_eq!((mob.extra.color, mob.extra.sheared), (color, sheared));
        }
        assert_eq!(game.world.game_time, clock);
        assert!(
            differences.is_empty(),
            "actual command wool-state differences: {differences:?}"
        );
    }
}

#[cfg(test)]
#[path = "loot_slot_tests.rs"]
pub(crate) mod loot_slot_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
