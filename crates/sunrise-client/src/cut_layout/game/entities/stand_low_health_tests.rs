use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::motion::PlayerMotion;

fn load_witness_26_3() -> String {
    let path = std::env::var("SUNRISE_STAND_LOW_HEALTH_WITNESS").unwrap_or_else(|_| {
        panic!(
            "SUNRISE_STAND_LOW_HEALTH_WITNESS is unset; run tools/test_stand_low_health_parity.py"
        )
    });
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {path} (record via tools/test_stand_low_health_parity.py): {e}"
        )
    })
}

struct WitnessRow<'a> {
    pub case: &'a str,
    nbt: &'a str,
    step: &'a str,
    alive: bool,
    health_bits: Option<u32>,
    death_time: Option<i16>,
    drop_count: usize,
    drop_item: Option<&'a str>,
}

fn parse_witness_26_3(text: &str) -> Vec<WitnessRow<'_>> {
    let mut rows = Vec::new();
    let mut header_seen = false;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !header_seen {
            assert_eq!(
                line,
                "case\tnbt\tstep\talive\thealth_bits\tdeath_time\tdrop_count\tdrop_item"
            );
            header_seen = true;
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 8, "bad witness row: {line}");
        let alive = match cols[3] {
            "1" => true,
            "0" => false,
            other => panic!("bad alive flag {other} in {line}"),
        };
        let health_bits = match cols[4] {
            "-" => None,
            hex => Some(u32::from_str_radix(hex, 16).unwrap()),
        };
        let death_time = match cols[5] {
            "-" => None,
            raw => Some(raw.parse::<i16>().unwrap()),
        };
        let drop_count = cols[6].parse::<usize>().unwrap();
        let drop_item = (cols[7] != "-").then_some(cols[7]);
        rows.push(WitnessRow {
            case: cols[0],
            nbt: cols[1],
            step: cols[2],
            alive,
            health_bits,
            death_time,
            drop_count,
            drop_item,
        });
    }
    assert!(header_seen && !rows.is_empty(), "empty 26.3 witness");
    rows
}

/// Vanilla 26.3 witness (#2692): low-health and dead armor stands match the real 26.3 oracle
/// recorded by `tools/test_stand_low_health_parity.py`.
#[test]
#[ignore = "run by tools/test_stand_low_health_parity.py"]
fn a_stand_breaks_on_low_health_only_through_damage_and_a_dead_one_goes_at_twenty() {
    run_a_stand_breaks_on_low_health_only_through_damage_and_a_dead_one_goes_at_twenty();
}

pub(crate) fn run_a_stand_breaks_on_low_health_only_through_damage_and_a_dead_one_goes_at_twenty() {
    let witness = load_witness_26_3();
    let rows = parse_witness_26_3(&witness);
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        let mut i = 0;
        while i < rows.len() {
            let case = rows[i].case;
            let nbt = rows[i].nbt;
            let mut end = i + 1;
            while end < rows.len() && rows[end].case == case {
                assert_eq!(rows[end].nbt, nbt, "case {case} changed summon NBT");
                end += 1;
            }
            game.armor_stands.clear();
            game.dropped.clear();
            game.summon_with_nbt("minecraft:armor_stand", [0.5, 64.0, 0.5], nbt)
                .unwrap();
            let id = game.armor_stands[0].id;
            let mut elapsed_ticks = 0u32;
            for row in &rows[i..end] {
                if let Some(tick) = row.step.strip_prefix("tick:") {
                    let target = tick.parse::<u32>().unwrap();
                    assert!(target >= elapsed_ticks, "{case} ticks went backwards");
                    for _ in elapsed_ticks..target {
                        game.tick_armor_stands().unwrap();
                    }
                    elapsed_ticks = target;
                } else if let Some(damage_type) = row.step.strip_prefix("damage:") {
                    game.command_damage_stand(id, damage_type, None);
                } else {
                    panic!("unknown witness step {} in {case}", row.step);
                }
                let stand = game.armor_stands.iter().find(|a| a.id == id);
                assert_eq!(
                    stand.is_some(),
                    row.alive,
                    "vanilla 26.3 {case} at {}: presence mismatch",
                    row.step
                );
                if let Some(a) = stand {
                    let saved = sunrise_sim::entity_nbt::armor_stand_to_nbt(
                        a,
                        [1, 2, 3, 4],
                        None,
                        game.res.items(),
                    );
                    let expected_bits = row.health_bits.expect("alive row missing health_bits");
                    let expected_death = row.death_time.expect("alive row missing death_time");
                    assert_eq!(
                        a.health.to_bits(),
                        expected_bits,
                        "vanilla 26.3 {case} at {}: health bits mismatch",
                        row.step
                    );
                    assert_eq!(
                        saved.get("Health"),
                        Some(&Tag::Float(expected_bits)),
                        "vanilla 26.3 {case} at {}: saved Health mismatch",
                        row.step
                    );
                    assert_eq!(
                        a.death_time, expected_death,
                        "vanilla 26.3 {case} at {}: death_time mismatch",
                        row.step
                    );
                    assert_eq!(
                        saved.get("DeathTime"),
                        Some(&Tag::Short(expected_death)),
                        "vanilla 26.3 {case} at {}: saved DeathTime mismatch",
                        row.step
                    );
                } else {
                    assert_eq!(row.health_bits, None, "{case} at {}", row.step);
                    assert_eq!(row.death_time, None, "{case} at {}", row.step);
                }
                assert_eq!(
                    game.dropped.len(),
                    row.drop_count,
                    "vanilla 26.3 {case} at {}: drop count mismatch",
                    row.step
                );
                assert_eq!(
                    game.dropped.first().map(|item| &*item.stack.item),
                    row.drop_item,
                    "vanilla 26.3 {case} at {}: drop item mismatch",
                    row.step
                );
            }
            i = end;
        }
    });
}
