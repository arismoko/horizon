use crate::game::native_hooks::on_chunk_unloaded_tests::*;
use std::time::Duration;
use sunrise_level::containers::BlockEntity;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

#[test]
#[ignore = "daily"]
fn each_mod_block_entity_of_an_unloading_chunk_is_told_once() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_hooks_server::on_chunk_unloaded_tests::each_mod_block_entity_of_an_unloading_chunk_is_told_once",
        "SUNRISE_ON_CHUNK_UNLOADED_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("unload_probe", Box::new(Probe))].into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let root = prepared_root();
    let saves = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("on-chunk-unloaded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);

    with_game_at(root, |game| {
        game.load_world(request(&saves, true)).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 1600 1600"));
        pump_until(game, true);
        game.tick_world(true).unwrap();
        for [x, y, z] in POTS {
            assert!(game.run_command(&format!("setblock {x} {y} {z} {POT}")));
        }
        let [x, y, z] = CHEST;
        assert!(game.run_command(&format!("setblock {x} {y} {z} minecraft:chest")));
        game.tick_world(true).unwrap();
        assert!(CALLS.lock().unwrap().is_empty(), "told before any unload");

        // The ticket goes: the chunk is saved, its parts are told, its block entities dropped.
        assert!(game.run_command("forceload remove 1600 1600"));
        pump_until(game, false);
        let mut calls = std::mem::take(&mut *CALLS.lock().unwrap());
        calls.sort();
        assert_eq!(
            calls,
            POTS.map(|pos| ("server", pos, POT.to_owned(), true))
                .to_vec(),
            "one call for each pot, none for the chest"
        );
        for [x, y, z] in POTS {
            assert!(game.world.block_entities.get(&(x, y, z)).is_none());
        }
        // A few more streaming rounds tell nothing again.
        for _ in 0..20 {
            game.server.streaming.wait(Duration::from_millis(5));
            game.server_update_world();
        }
        assert!(CALLS.lock().unwrap().is_empty(), "told twice");

        // Back from its save: both pots, without what was written while it unloaded.
        assert!(game.run_command("forceload add 1600 1600"));
        pump_until(game, true);
        for [x, y, z] in POTS {
            let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(x, y, z)) else {
                panic!("pot at {x} {y} {z} was not saved");
            };
            assert_eq!(entity.id(), POT);
            assert!(!entity.nbt.contains_key(&"written_on_unload".into()));
        }

        // Closing the world, its chunks with it, tells nothing.
        game.finish_server_save(false, false).unwrap();
        game.world.chunks.clear();
        assert!(
            game.world.chunks.is_empty(),
            "the world's chunks went with it"
        );
        assert!(
            CALLS.lock().unwrap().is_empty(),
            "told when the world closed"
        );
    });
    let _ = std::fs::remove_dir_all(&saves);
}
