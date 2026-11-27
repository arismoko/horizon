use serde_json::Value;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::parse_snbt;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

fn message_text(row: &Value) -> String {
    row["messages"][0]
        .as_array()
        .expect("witness message spans")
        .iter()
        .map(|span| span[0].as_str().expect("witness span text"))
        .collect()
}

fn parse_summon(cmd: &str) -> ([f64; 3], &str) {
    let rest = cmd
        .strip_prefix("/summon minecraft:falling_block ")
        .expect("falling_block summon");
    let brace = rest.find('{').expect("summon NBT");
    let (coords, nbt) = rest.split_at(brace);
    let mut parts = coords.split_whitespace();
    let pos = [
        parts.next().expect("x").parse::<f64>().unwrap(),
        parts.next().expect("y").parse::<f64>().unwrap(),
        parts.next().expect("z").parse::<f64>().unwrap(),
    ];
    assert!(parts.next().is_none(), "extra summon coords: {coords}");
    (pos, nbt)
}

/// #2363 (vanilla 26.3): a falling block resting in a `moving_piston` cell stays across ticks.
/// Matches the witness `tools/test_falling_moving_piston_parity.py` records.
#[test]
#[ignore = "run by tools/test_falling_moving_piston_parity.py"]
fn a_falling_block_resting_in_a_moving_piston_cell_stays() {
    run_a_falling_block_resting_in_a_moving_piston_cell_stays();
}

pub(crate) fn run_a_falling_block_resting_in_a_moving_piston_cell_stays() {
    let path = std::env::var("SUNRISE_FALLING_MOVING_PISTON_WITNESS").expect(
        "run through tools/test_falling_moving_piston_parity.py, which records the witness",
    );
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_FALLING_MOVING_PISTON_WITNESS={path} \
             (run through tools/test_falling_moving_piston_parity.py): {err}"
        )
    });
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 31 15"));
        for command in [
            "fill 0 209 0 4 209 4 minecraft:stone",
            "setblock 2 210 2 minecraft:moving_piston",
        ] {
            assert!(game.run_command(command), "{command}");
        }

        let mut lines = witness.lines().filter(|line| !line.trim().is_empty());
        let header: Value = serde_json::from_str(lines.next().expect("witness header")).unwrap();
        assert_eq!(header["minecraft"].as_str(), Some("26.3"));

        let mut current_id: Option<u32> = None;
        let mut checked = 0;
        for (index, line) in lines.enumerate() {
            let row: Value = serde_json::from_str(line).unwrap();
            let cmd = row["cmd"].as_str().expect("witness cmd");
            if cmd.starts_with("/summon minecraft:falling_block ") {
                if let Some(prev) = current_id.take() {
                    game.falling.retain(|f| f.id != prev);
                }
                let (pos, nbt) = parse_summon(cmd);
                let target = game
                    .summon_with_nbt("minecraft:falling_block", pos, nbt)
                    .unwrap()
                    .unwrap();
                let Target::Falling(id) = target else {
                    panic!("{target:?}");
                };
                current_id = Some(id);
            } else if let Some(rest) = cmd.strip_prefix("/data get entity ") {
                let (_, field) = rest.rsplit_once(' ').expect("data get field");
                let id = current_id.expect("summon before probe");
                if field == "Pos" {
                    let i = game
                        .falling
                        .iter()
                        .position(|f| f.id == id)
                        .expect("the block stays");
                    game.tick_one_falling(i, &[]).unwrap();
                }
                let text = message_text(&row);
                let (_, snbt) = text
                    .split_once(" has the following entity data: ")
                    .expect("entity data feedback");
                let (expected, consumed) = parse_snbt(snbt).unwrap();
                assert_eq!(consumed, snbt.len(), "record {index}: {snbt}");
                let saved = game
                    .command_target_nbt(Target::Falling(id))
                    .expect("the block stays");
                // Tag::Double compares raw IEEE-754 u64 bits.
                assert_eq!(saved.get(field), Some(&expected), "record {index}: {cmd}");
                checked += 1;
            } else if cmd == "/execute if block 2 210 2 minecraft:moving_piston" {
                let expected = row["results"][0][0].as_bool().expect("block check result");
                let actual = &*game.res.facts().get(game.world.block(2, 210, 2)).name
                    == "minecraft:moving_piston";
                assert_eq!(actual, expected, "record {index}: {cmd}");
                checked += 1;
            } else if cmd.starts_with("/execute if entity @e[type=minecraft:item,") {
                let expected = row["results"][0][0].as_bool().expect("item check result");
                assert_eq!(!game.dropped.is_empty(), expected, "record {index}: {cmd}");
                checked += 1;
            } else {
                panic!("unexpected witness record {index}: {cmd}");
            }
        }
        assert!(checked > 0, "witness must check recorded assertions");
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
