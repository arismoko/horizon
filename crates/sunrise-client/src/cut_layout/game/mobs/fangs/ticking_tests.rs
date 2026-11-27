use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::motion::PlayerMotion;

fn load_witness() -> TickingGateWitness {
    let path = std::env::var_os("SUNRISE_FANGS_TICKING_GATE_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_FANGS_TICKING_GATE_WITNESS is unset; run tools/test_fangs_ticking_gate_parity.py"
        )
    });
    let path = PathBuf::from(path);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_fangs_ticking_gate_parity.py): {e}",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("parse 26.3 ticking-gate witness")
}

#[derive(Deserialize)]
struct TickingGateWitness {
    game_version: String,
    player_pos: [f64; 3],
    view_distance: i32,
    simulation_distance: i32,
    cases: Vec<TickingGateCase>,
    scheduled_ticket_pause: ScheduledTicketPauseWitness,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum TickingGateCase {
    #[serde(rename = "evoker_fangs")]
    EvokerFangs {
        name: String,
        pos: [f64; 3],
        forced_chunks: Vec<[i32; 2]>,
        initial_warmup: i32,
        ticks: u32,
        expected_present: bool,
        expected_warmup: Option<i32>,
    },
    #[serde(rename = "ominous_item_spawner")]
    OminousItemSpawner {
        name: String,
        pos: [f64; 3],
        forced_chunks: Vec<[i32; 2]>,
        item: String,
        spawn_item_after_ticks: i32,
        ticks: u32,
        expected_present: bool,
        expected_spawned_items: usize,
    },
}

#[derive(Deserialize)]
struct ScheduledTicketPauseWitness {
    target_chunk: [i32; 2],
    control_chunk: [i32; 2],
    command_block_pos: [i32; 3],
    target_fangs_pos: [f64; 3],
    target_spawner_pos: [f64; 3],
    control_fangs_pos: [f64; 3],
    control_spawner_pos: [f64; 3],
    initial_warmup: i32,
    item: String,
    spawn_item_after_ticks: i32,
    stages: Vec<ScheduledTicketPauseStage>,
}

#[derive(Deserialize)]
struct ScheduledTicketPauseStage {
    name: String,
    action: String,
    ticks: u32,
    expected_command_block_success: Option<i32>,
    expected_target_loaded: bool,
    expected_control_loaded: bool,
    expected_target_warmup: i32,
    expected_target_spawner_present: bool,
    expected_target_items: usize,
    expected_control_warmup: i32,
    expected_control_spawner_present: bool,
    expected_control_items: usize,
}

/// #2361 (vanilla 26.3): evoker fangs and ominous item spawners match the ticking-chunk gate witness `tools/test_fangs_ticking_gate_parity.py` records.
#[test]
#[ignore = "run by tools/test_fangs_ticking_gate_parity.py"]
fn fangs_and_an_ominous_item_spawner_outside_the_ticking_chunks_do_not_tick() {
    run_fangs_and_an_ominous_item_spawner_outside_the_ticking_chunks_do_not_tick();
}

pub(crate) fn run_fangs_and_an_ominous_item_spawner_outside_the_ticking_chunks_do_not_tick() {
    let witness = load_witness();
    assert_eq!(witness.game_version, "26.3");
    assert!(!witness.cases.is_empty());

    with_game(|game| {
        game.server.settings.render_distance = witness.view_distance;
        game.server.settings.simulation_distance = witness.simulation_distance;
        let [px, py, pz] = witness.player_pos;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(px, py, pz)).unwrap());

        for case in &witness.cases {
            game.fangs.list.clear();
            game.ominous_spawners.clear();
            game.dropped.clear();
            game.server.streaming.forced.clear();

            match case {
                TickingGateCase::EvokerFangs {
                    name,
                    pos,
                    forced_chunks,
                    initial_warmup,
                    ticks,
                    expected_present,
                    expected_warmup,
                } => {
                    for &[cx, cz] in forced_chunks {
                        game.server.streaming.forced.insert((cx, cz));
                    }
                    game.update_chunk_tickets();
                    game.add_fangs(*pos, 0.0, *initial_warmup, 0);
                    for _ in 0..*ticks {
                        game.tick_fangs();
                    }
                    let fangs = game.fangs.list.first();
                    assert_eq!(
                        game.fangs.list.len(),
                        usize::from(*expected_present),
                        "case {name}: fangs count"
                    );
                    assert_eq!(
                        fangs.map(|f| f.warmup),
                        *expected_warmup,
                        "case {name}: fangs warmup"
                    );
                    if let Some(fangs) = fangs {
                        assert_eq!(
                            fangs.pos.map(f64::to_bits),
                            pos.map(f64::to_bits),
                            "case {name}: fangs position bits"
                        );
                    }
                }
                TickingGateCase::OminousItemSpawner {
                    name,
                    pos,
                    forced_chunks,
                    item,
                    spawn_item_after_ticks,
                    ticks,
                    expected_present,
                    expected_spawned_items,
                } => {
                    for &[cx, cz] in forced_chunks {
                        game.server.streaming.forced.insert((cx, cz));
                    }
                    game.update_chunk_tickets();
                    game.add_ominous_item_spawner(
                        ItemStack::new(item, 1),
                        *pos,
                        *spawn_item_after_ticks,
                    );
                    for _ in 0..*ticks {
                        game.tick_ominous_item_spawners();
                    }
                    assert_eq!(
                        game.ominous_spawners.len(),
                        usize::from(*expected_present),
                        "case {name}: spawner count"
                    );
                    if let Some(spawner) = game.ominous_spawners.first() {
                        assert_eq!(
                            spawner.position.map(f64::to_bits),
                            pos.map(f64::to_bits),
                            "case {name}: spawner position bits"
                        );
                    }
                    assert_eq!(
                        game.dropped.len(),
                        *expected_spawned_items,
                        "case {name}: spawned items"
                    );
                    for dropped in game.dropped.iter() {
                        assert_eq!(
                            dropped.stack.item.as_ref(),
                            item.as_str(),
                            "case {name}: spawned item id"
                        );
                        assert_eq!(dropped.stack.count, 1, "case {name}: spawned item count");
                    }
                }
            }
        }
    });
}

/// #2361 (vanilla 26.3): removing a resident chunk's forced ticket in a scheduled block tick pauses fangs and spawners on that same tick until resumed.
#[test]
#[ignore = "run by tools/test_fangs_ticking_gate_parity.py"]
fn fangs_and_an_ominous_item_spawner_pause_on_scheduled_ticket_removal_and_resume_26_3() {
    run_fangs_and_an_ominous_item_spawner_pause_on_scheduled_ticket_removal_and_resume_26_3();
}

pub(crate) fn run_fangs_and_an_ominous_item_spawner_pause_on_scheduled_ticket_removal_and_resume_26_3()
 {
    let witness = load_witness();
    assert_eq!(witness.game_version, "26.3");
    let pause = &witness.scheduled_ticket_pause;
    assert!(!pause.stages.is_empty());

    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.server.settings.render_distance = witness.view_distance;
        game.server.settings.simulation_distance = witness.simulation_distance;
        let [px, py, pz] = witness.player_pos;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(px, py, pz)).unwrap());

        let [tcx, tcz] = pause.target_chunk;
        let [ccx, ccz] = pause.control_chunk;
        let min_cx = tcx.min(ccx) - 1;
        let max_cx = tcx.max(ccx) + 1;
        let min_cz = tcz.min(ccz) - 1;
        let max_cz = tcz.max(ccz) + 1;
        for cx in min_cx..=max_cx {
            for cz in min_cz..=max_cz {
                let mut chunk = Chunk::empty(cx, cz);
                chunk.lit = true;
                game.world.chunks.insert((cx, cz), Arc::new(chunk));
                let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(cx, cz));
            }
        }

        let (tx0, tz0) = (tcx * 16, tcz * 16);
        let (cx0, cz0) = (ccx * 16, ccz * 16);
        let [cbx, cby, cbz] = pause.command_block_pos;
        assert!(game.run_command(&format!("forceload add {tx0} {tz0}")));
        assert!(game.run_command(&format!("forceload add {cx0} {cz0}")));
        assert!(game.run_command(&format!(
            "setblock {} {cby} {} minecraft:stone",
            pause.target_spawner_pos[0].floor() as i32,
            pause.target_spawner_pos[2].floor() as i32
        )));
        assert!(game.run_command(&format!(
            "setblock {} {cby} {} minecraft:stone",
            pause.control_spawner_pos[0].floor() as i32,
            pause.control_spawner_pos[2].floor() as i32
        )));
        game.update_chunk_tickets();
        game.sync_tick_chunks();
        game.start_ticking_chunks();

        game.add_fangs(pause.target_fangs_pos, 0.0, pause.initial_warmup, 0);
        game.add_ominous_item_spawner(
            ItemStack::new(&pause.item, 1),
            pause.target_spawner_pos,
            pause.spawn_item_after_ticks,
        );
        game.add_fangs(pause.control_fangs_pos, 0.0, pause.initial_warmup, 0);
        game.add_ominous_item_spawner(
            ItemStack::new(&pause.item, 1),
            pause.control_spawner_pos,
            pause.spawn_item_after_ticks,
        );

        let in_chunk = |pos: Vec3, chunk: [i32; 2]| {
            ((pos.x.floor() as i32) >> 4) == chunk[0] && ((pos.z.floor() as i32) >> 4) == chunk[1]
        };

        for stage in &pause.stages {
            match stage.action.as_str() {
                "schedule_remove_and_tick" => {
                    assert!(
                        game.run_command(&format!(
                            "setblock {cbx} {cby} {cbz} minecraft:command_block{{auto:1b,Command:\"forceload remove {tx0} {tz0}\"}}"
                        )),
                        "stage {}: place command block",
                        stage.name
                    );
                }
                "forceload_add" => {
                    assert!(
                        game.run_command(&format!("forceload add {tx0} {tz0}")),
                        "stage {}: resume forceload",
                        stage.name
                    );
                    game.update_chunk_tickets();
                }
                "tick" => {}
                other => panic!("unexpected stage action: {other}"),
            }

            for _ in 0..stage.ticks {
                game.world.game_time += 1;
                game.tick_world(true).unwrap();
            }

            if let Some(expected_success) = stage.expected_command_block_success {
                let Some(BlockEntity::CommandBlock(block)) =
                    game.world.block_entities.get(&(cbx, cby, cbz))
                else {
                    panic!("stage {}: command block missing", stage.name);
                };
                assert_eq!(
                    block.success_count, expected_success,
                    "stage {}: command block SuccessCount",
                    stage.name
                );
            }

            let ticking = game.entity_ticking_area();
            assert_eq!(
                ticking(tx0, tz0),
                stage.expected_target_loaded,
                "stage {}: target entity-ticking",
                stage.name
            );
            assert_eq!(
                ticking(cx0, cz0),
                stage.expected_control_loaded,
                "stage {}: control entity-ticking",
                stage.name
            );
            assert!(
                game.world.chunks.contains_key(&(tcx, tcz)),
                "stage {}: target chunk stayed resident",
                stage.name
            );

            let t_fangs = game
                .fangs
                .list
                .iter()
                .find(|f| f.pos.map(f64::to_bits) == pause.target_fangs_pos.map(f64::to_bits));
            assert_eq!(
                t_fangs.map(|f| f.warmup),
                Some(stage.expected_target_warmup),
                "stage {}: target fangs warmup",
                stage.name
            );
            let t_spawner = game.ominous_spawners.iter().any(|s| {
                s.position.map(f64::to_bits) == pause.target_spawner_pos.map(f64::to_bits)
            });
            assert_eq!(
                t_spawner, stage.expected_target_spawner_present,
                "stage {}: target spawner presence",
                stage.name
            );
            let t_items: Vec<_> = game
                .dropped
                .iter()
                .filter(|d| in_chunk(d.position, pause.target_chunk))
                .collect();
            assert_eq!(
                t_items.len(),
                stage.expected_target_items,
                "stage {}: target spawned item count",
                stage.name
            );
            for dropped in t_items {
                assert_eq!(dropped.stack.item.as_ref(), pause.item.as_str());
                assert_eq!(dropped.stack.count, 1);
            }

            let c_fangs = game
                .fangs
                .list
                .iter()
                .find(|f| f.pos.map(f64::to_bits) == pause.control_fangs_pos.map(f64::to_bits));
            assert_eq!(
                c_fangs.map(|f| f.warmup),
                Some(stage.expected_control_warmup),
                "stage {}: control fangs warmup",
                stage.name
            );
            let c_spawner = game.ominous_spawners.iter().any(|s| {
                s.position.map(f64::to_bits) == pause.control_spawner_pos.map(f64::to_bits)
            });
            assert_eq!(
                c_spawner, stage.expected_control_spawner_present,
                "stage {}: control spawner presence",
                stage.name
            );
            let c_items: Vec<_> = game
                .dropped
                .iter()
                .filter(|d| in_chunk(d.position, pause.control_chunk))
                .collect();
            assert_eq!(
                c_items.len(),
                stage.expected_control_items,
                "stage {}: control spawned item count",
                stage.name
            );
            for dropped in c_items {
                assert_eq!(dropped.stack.item.as_ref(), pause.item.as_str());
                assert_eq!(dropped.stack.count, 1);
            }
        }
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
