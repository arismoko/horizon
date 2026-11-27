use crate::game::server_side::commands::summon_bolt_tests::ORACLE_PLAYER_UUID;
use crate::game::server_side::commands::summon_bolt_tests::tag_contains_bolt;
use serde_json::{Value, json};
use std::collections::HashSet;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;
use sunrise_core::random::RandomSource;
use sunrise_gameplay::game::command_harness::spans;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) fn run_a_bolt_summoned_with_nbt_strikes_and_goes() {
    let witness_path = std::env::var_os("SUNRISE_SUMMON_BOLT_NBT_WITNESS").unwrap_or_else(|| {
        panic!("SUNRISE_SUMMON_BOLT_NBT_WITNESS is unset; run tools/test_summon_bolt_nbt_parity.py")
    });
    let witness_path = std::path::PathBuf::from(witness_path);
    let witness_path = if witness_path.is_dir() {
        witness_path.join("summon_bolt_nbt.json")
    } else {
        witness_path
    };
    let raw = std::fs::read_to_string(&witness_path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (run tools/test_summon_bolt_nbt_parity.py): {e}",
            witness_path.display()
        )
    });

    // Opt in to deterministic thread seeds only in a fresh child process.
    if run_fresh_cases(
        "cut_layout::game::server_side::commands::summon_bolt_tests::a_bolt_summoned_with_nbt_strikes_and_goes",
        "SUNRISE_TEST_SEEDS",
        &["1"],
    ) {
        return;
    }

    let witness: Value = serde_json::from_str(&raw).expect("valid 26.3 witness JSON");
    assert_eq!(witness["minecraft"], "26.3");
    let steps = witness["steps"].as_array().unwrap();
    assert!(!steps.is_empty());

    let _language = sunrise_game_data::lang::isolate_publication(
        sunrise_game_data::lang::Language::of("en_us", []),
    );
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(15, 0));
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(16, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 230.0, 8.5)).unwrap());
        bind_local_player(game, 1, ORACLE_PLAYER_UUID);
        game.creative = true;
        game.ui.difficulty = 2;
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0"));
        game.update_chunk_tickets();
        for setup_cmd in [
            "gamerule spawn_mobs false",
            "gamerule mob_drops false",
            "gamerule entity_drops false",
            "gamerule advance_time false",
            "gamerule advance_weather false",
        ] {
            let _ = game.run_command(setup_cmd);
        }

        let mut saw_live_bolt_save_check = false;
        let mut saw_mounted_bolt_save_check = false;
        for (index, step) in steps.iter().enumerate() {
            match step["op"].as_str().unwrap() {
                "sync_randoms" => {
                    let uniquifier = step["uniquifier"].as_i64().unwrap();
                    let entity_id = step["entity_id"].as_i64().unwrap();
                    let level_seed = step["level_seed"].as_i64().unwrap();
                    sunrise_level::random_source::set_thread_uniquifier(uniquifier).unwrap();
                    game.server.next_entity = u32::try_from(entity_id).unwrap();
                    game.world.random.set_seed(level_seed);
                }
                "wait_chunks" => {
                    for pair in step["chunks"].as_array().unwrap() {
                        let cx = pair[0].as_i64().unwrap() as i32;
                        let cz = pair[1].as_i64().unwrap() as i32;
                        prepare_entity_ticking_neighborhood(game, ChunkPos::new(cx, cz));
                    }
                    game.update_chunk_tickets();
                    game.spawn_loaded_entities();
                }
                "cmd" => {
                    let cmd = step["cmd"].as_str().unwrap();
                    let line = cmd.strip_prefix('/').unwrap_or(cmd);
                    game.server.chat_capture = Some(Vec::new());
                    game.server.cmds.0.results = Some(Vec::new());
                    game.as_console(|g| g.run_command(line));
                    let messages = game.server.chat_capture.take().unwrap_or_default();
                    let results = game.server.cmds.0.results.take().unwrap_or_default();
                    if step["record"].as_bool() != Some(false) {
                        let actual_messages = Value::Array(messages.iter().map(spans).collect());
                        let actual_results =
                            Value::Array(results.iter().map(|(s, v)| json!([s, v])).collect());
                        assert_eq!(
                            actual_messages, step["messages"],
                            "step {index} ({cmd}): messages differ from 26.3 witness"
                        );
                        assert_eq!(
                            actual_results, step["results"],
                            "step {index} ({cmd}): results differ from 26.3 witness"
                        );
                    }
                    assert!(
                        game.server
                            .others
                            .0
                            .iter()
                            .all(|o| o.kind != "minecraft:lightning_bolt"),
                        "step {index} ({cmd}): lightning_bolt must never be stored in others"
                    );
                    if !game.bolts.is_empty() {
                        let keys: HashSet<(i32, i32)> = game
                            .bolts
                            .iter()
                            .map(|b| ((b.at[0].floor() as i32) >> 4, (b.at[2].floor() as i32) >> 4))
                            .collect();
                        let saved = game.entity_chunks(&keys, false);
                        for (chunk_key, tags) in &saved {
                            for tag in tags {
                                assert!(
                                    !tag_contains_bolt(tag),
                                    "step {index} ({cmd}): chunk {chunk_key:?} saved a lightning_bolt: {tag:?}"
                                );
                            }
                        }
                        saw_live_bolt_save_check = true;
                        if game.bolts.iter().any(|b| b.mount.is_some()) {
                            saw_mounted_bolt_save_check = true;
                        }
                    }
                }
                "tick" => {
                    let n = step["n"].as_u64().unwrap();
                    for _ in 0..n {
                        game.tick_server(false).unwrap();
                    }
                }
                other => panic!("step {index}: unexpected witness op {other}"),
            }
        }
        assert!(
            saw_live_bolt_save_check,
            "witness must exercise live bolt save exclusion"
        );
        assert!(
            saw_mounted_bolt_save_check,
            "witness must exercise mounted bolt save exclusion"
        );
        assert!(
            game.bolts.is_empty(),
            "bolts must be gone after 26.3 witness"
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
