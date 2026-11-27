use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::{Tag, parse_snbt};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::server_side::commands::data::DataRef,
    sunrise_gameplay_common::game::commands_common::Target,
};

#[derive(Deserialize)]
struct Witness {
    minecraft: String,
    fields: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    in_water: bool,
    nbt: String,
    snapshots: Vec<Snapshot>,
}

#[derive(Deserialize)]
struct Snapshot {
    tick: usize,
    values: BTreeMap<String, String>,
}

fn parse_expected(raw: &str) -> Tag {
    let (tag, used) = parse_snbt(raw).unwrap();
    assert_eq!(used, raw.len(), "trailing SNBT in witness value {raw:?}");
    tag
}

fn tick(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.handle_entity_portals();
    game.tick_armor_stands().unwrap();
}

const WITNESS_ENV: &str = "SUNRISE_STAND_LIVING_COUNTERS_WITNESS";

fn load_witness() -> Witness {
    let raw_path = std::env::var_os(WITNESS_ENV).unwrap_or_else(|| {
        panic!(
            "{WITNESS_ENV} is unset; run tools/test_stand_living_counters_parity.py to record the witness"
        )
    });
    let mut path = std::path::PathBuf::from(raw_path);
    if path.is_dir() {
        path.push("stand_living_counters.json");
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_stand_living_counters_parity.py): {e}",
            path.display()
        )
    });
    serde_json::from_str(&raw).expect("valid 26.3 armor stand living counters witness JSON")
}

/// #2678: vanilla 26.3 witness for armor stand living counters in air and water, recorded by `tools/test_stand_living_counters_parity.py`.
#[test]
#[ignore = "run by tools/test_stand_living_counters_parity.py"]
fn an_armor_stand_ticks_its_living_counters() {
    run_an_armor_stand_ticks_its_living_counters();
}

pub(crate) fn run_an_armor_stand_ticks_its_living_counters() {
    let witness = load_witness();
    assert_eq!(witness.minecraft, "26.3");
    assert!(!witness.cases.is_empty());
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        for x in 6..=10 {
            for y in 64..=66 {
                for z in 6..=10 {
                    game.world.set(x, y, z, water).unwrap();
                }
            }
        }
        game.update_chunk_tickets();
        let mut dry_index = 0u32;
        let mut water_index = 0u32;
        let actors: Vec<(&Case, Target)> = witness
            .cases
            .iter()
            .map(|case| {
                let pos = if case.in_water {
                    let i = water_index;
                    water_index += 1;
                    [7.5 + 2.0 * f64::from(i), 64.0, 8.5]
                } else {
                    let i = dry_index;
                    dry_index += 1;
                    [1.5 + f64::from(i), 64.0, 2.5]
                };
                let stand = game
                    .summon_with_nbt("minecraft:armor_stand", pos, &case.nbt)
                    .unwrap()
                    .unwrap();
                (case, stand)
            })
            .collect();
        let mut elapsed = 0usize;
        let checkpoints: Vec<usize> = witness.cases[0].snapshots.iter().map(|s| s.tick).collect();
        for (snap_idx, &target_tick) in checkpoints.iter().enumerate() {
            for _ in elapsed..target_tick {
                tick(game);
            }
            elapsed = target_tick;
            for (case, stand) in actors.iter().copied() {
                let snapshot = &case.snapshots[snap_idx];
                assert_eq!(snapshot.tick, target_tick);
                let tag = game.data_of(&DataRef::Entity(stand)).unwrap();
                for field in &witness.fields {
                    let expected = parse_expected(snapshot.values.get(field).unwrap());
                    assert_eq!(
                        tag.get(field.as_str()),
                        Some(&expected),
                        "case={} tick={} field={field}",
                        case.name,
                        target_tick,
                    );
                }
            }
        }
    });
}
