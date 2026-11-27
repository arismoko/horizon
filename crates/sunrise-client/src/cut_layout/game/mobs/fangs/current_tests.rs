use std::collections::BTreeMap;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay::game::commands::data::DataRef;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

fn read_witness() -> String {
    let raw = std::env::var_os("SUNRISE_FANGS_FLUID_CURRENT_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_FANGS_FLUID_CURRENT_WITNESS is unset; run through tools/test_fangs_fluid_current_parity.py, which records the 26.3 witness"
        )
    });
    let path = std::path::PathBuf::from(raw);
    let file = if path.is_dir() {
        path.join("fangs_fluid_current.tsv")
    } else {
        path
    };
    std::fs::read_to_string(&file).unwrap_or_else(|err| {
        panic!(
            "failed to read {} (run through tools/test_fangs_fluid_current_parity.py): {err}",
            file.display()
        )
    })
}

fn load_witness(witness: &str) -> BTreeMap<(&str, usize), [u64; 3]> {
    let mut rows = BTreeMap::new();
    for line in witness.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let case = cols.next().expect("case column");
        let tick: usize = cols.next().expect("tick column").parse().expect("tick");
        let parse_bits = |s: &str| u64::from_str_radix(s, 16).expect("hex f64 bits");
        let bx = parse_bits(cols.next().expect("mx_bits"));
        let by = parse_bits(cols.next().expect("my_bits"));
        let bz = parse_bits(cols.next().expect("mz_bits"));
        assert_eq!(cols.count(), 3, "expected 8 columns in {line}");
        assert!(
            rows.insert((case, tick), [bx, by, bz]).is_none(),
            "duplicate witness row ({case}, {tick})"
        );
    }
    rows
}

fn motion_bits(v: Vec3) -> [u64; 3] {
    let canon = |d: f64| if d == 0.0 { 0 } else { d.to_bits() };
    [canon(v.x), canon(v.y), canon(v.z)]
}

fn check_fangs_motion(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    name: &str,
    uuid: [i32; 4],
    id: u32,
    tick: usize,
    want: [u64; 3],
) {
    let expected_tag = Some(Tag::List(want.into_iter().map(Tag::Double).collect()));
    let (live_motion, saved) = {
        let fangs = game
            .fangs
            .list
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("missing fangs {name}"));
        (fangs.motion, game.fangs_to_nbt(fangs, uuid))
    };
    assert_eq!(
        motion_bits(live_motion),
        want,
        "live motion bits mismatch for {name} at tick {tick}: {live_motion:?}"
    );
    assert_eq!(
        saved.get("Motion").cloned(),
        expected_tag,
        "saved Motion NBT bits mismatch for {name} at tick {tick}"
    );
    assert_eq!(
        motion_bits(sunrise_sim::entity_nbt::motion(&saved)),
        want,
        "re-read motion bits mismatch for {name} at tick {tick}"
    );
    let target = game
        .target_with_uuid(uuid)
        .unwrap_or_else(|| panic!("missing command target for {name}"));
    let live_nbt = game.data_of(&DataRef::Entity(target)).unwrap();
    assert_eq!(
        live_nbt.get("Motion").cloned(),
        expected_tag,
        "/data get Motion bits mismatch for {name} at tick {tick}"
    );
}

/// #2362 (vanilla 26.3): EvokerFangs fluid-current live and saved Motion match
/// the 26.3 witness recorded by `tools/test_fangs_fluid_current_parity.py`
/// bit-for-bit across water, slow/fast lava, widened float height, and
/// tick-eligibility guards.
#[test]
#[ignore = "run by tools/test_fangs_fluid_current_parity.py"]
fn fangs_in_a_current_are_pushed_and_fangs_in_still_water_are_not() {
    run_fangs_in_a_current_are_pushed_and_fangs_in_still_water_are_not();
}

pub(crate) fn run_fangs_in_a_current_are_pushed_and_fangs_in_still_water_are_not() {
    let witness = read_witness();
    assert!(
        witness.starts_with("# minecraft 26.3 "),
        "witness must be tagged with vanilla 26.3"
    );
    let expected = load_witness(&witness);
    let mut checked = 0usize;

    // Overworld cases, widened 0.8F height boundary, and non-ticking/late-admission guards.
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        // Loaded but non-entity-ticking chunk neighborhood well outside simulation distance.
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(20, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 15 15"));
        assert!(
            !game.entity_ticking_area()(326, 3),
            "chunk (20, 0) must be loaded without entity ticking"
        );
        for command in [
            "fill 0 199 0 15 199 15 minecraft:stone",
            "fill 0 200 0 15 202 15 minecraft:barrier",
            "fill 0 200 0 15 202 15 minecraft:air",
            // Lava flowing north (-z) from its source through the fangs' cell.
            "setblock 2 200 4 minecraft:lava",
            "setblock 2 200 3 minecraft:lava[level=2]",
            // Water flowing north (-z) from its source through the fangs' cell.
            "setblock 6 200 4 minecraft:water",
            "setblock 6 200 3 minecraft:water[level=1]",
            // Diagonal water current (+x, -z) from a south source and an east level-3 flow.
            "setblock 10 200 4 minecraft:water",
            "setblock 11 200 3 minecraft:water[level=3]",
            "setblock 10 200 3 minecraft:water[level=1]",
            // Still water pools walled in on every side.
            "fill 1 200 9 3 200 11 minecraft:stone",
            "setblock 2 200 10 minecraft:water",
            "fill 5 200 9 7 200 11 minecraft:stone",
            "setblock 6 200 10 minecraft:water",
            // Upper-cell-only flowing water at y=201 (air at y=200), testing widened 0.8F height.
            "setblock 13 200 4 minecraft:stone",
            "setblock 13 200 2 minecraft:stone",
            "setblock 12 201 3 minecraft:stone",
            "setblock 14 201 3 minecraft:stone",
            "setblock 13 201 4 minecraft:water",
            "setblock 13 201 3 minecraft:water[level=1]",
            // Flowing water in the loaded, non-entity-ticking chunk (20, 0).
            "fill 325 199 2 327 199 5 minecraft:stone",
            "setblock 326 200 4 minecraft:water",
            "setblock 326 200 3 minecraft:water[level=1]",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        let cases = [
            (
                "lava_north",
                [2362, 0, 0, 1],
                "summon minecraft:evoker_fangs 2.5 200 3.5 {UUID:[I;2362,0,0,1],Warmup:100}",
            ),
            (
                "water_north",
                [2362, 0, 0, 2],
                "summon minecraft:evoker_fangs 6.5 200 3.5 {UUID:[I;2362,0,0,2],Warmup:100}",
            ),
            (
                "water_diagonal",
                [2362, 0, 0, 3],
                "summon minecraft:evoker_fangs 10.5 200 3.5 {UUID:[I;2362,0,0,3],Warmup:100}",
            ),
            (
                "still_water",
                [2362, 0, 0, 4],
                "summon minecraft:evoker_fangs 2.5 200 10.5 {UUID:[I;2362,0,0,4],Warmup:100}",
            ),
            (
                "still_water_loaded",
                [2362, 0, 0, 5],
                "summon minecraft:evoker_fangs 6.5 200 10.5 {UUID:[I;2362,0,0,5],Warmup:100,Motion:[0.125d,-0.25d,0.5d]}",
            ),
            (
                "dry_air_loaded",
                [2362, 0, 0, 6],
                "summon minecraft:evoker_fangs 10.5 200 10.5 {UUID:[I;2362,0,0,6],Warmup:100,Motion:[-0.0625d,0.03125d,0.25d]}",
            ),
            (
                "water_north_loaded",
                [2362, 0, 0, 7],
                "summon minecraft:evoker_fangs 6.5 200 3.5 {UUID:[I;2362,0,0,7],Warmup:100,Motion:[0.05d,0.0d,-0.02d]}",
            ),
            (
                "lava_north_loaded",
                [2362, 0, 0, 8],
                "summon minecraft:evoker_fangs 2.5 200 3.5 {UUID:[I;2362,0,0,8],Warmup:100,Motion:[0.05d,0.0d,-0.02d]}",
            ),
            (
                "upper_cell_float_height",
                [2362, 0, 0, 9],
                "summon minecraft:evoker_fangs 13.5 200.20099999 3.5 {UUID:[I;2362,0,0,9],Warmup:100}",
            ),
            (
                "upper_cell_just_below",
                [2362, 0, 0, 10],
                "summon minecraft:evoker_fangs 13.5 200.20099998 3.5 {UUID:[I;2362,0,0,10],Warmup:100}",
            ),
        ];
        let mut spawned = Vec::with_capacity(cases.len());
        for (name, uuid, command) in cases {
            assert!(game.run_command(command), "{command}");
            let id = game.fangs.list.last().expect("summoned fangs").id;
            spawned.push((name, uuid, id));
        }
        assert!(game.run_command(
            "summon minecraft:evoker_fangs 326.5 200 3.5 {UUID:[I;2362,0,1,1],Warmup:100}"
        ));
        let non_ticking_id = game.fangs.list.last().expect("non-ticking fangs").id;

        for tick in 0..=2usize {
            if tick > 0 {
                game.tick_world(true).unwrap();
            }
            for &(name, uuid, id) in &spawned {
                let want = *expected
                    .get(&(name, tick))
                    .unwrap_or_else(|| panic!("missing witness row ({name}, {tick})"));
                check_fangs_motion(game, name, uuid, id, tick, want);
                checked += 1;
            }
            // Non-entity-ticking loaded chunk must never push or warm down fangs.
            let non_ticking = game
                .fangs
                .list
                .iter()
                .find(|f| f.id == non_ticking_id)
                .unwrap();
            assert_eq!(
                motion_bits(non_ticking.motion),
                [0; 3],
                "non-entity-ticking fangs must not be pushed at tick {tick}"
            );
            assert_eq!(
                non_ticking.warmup, 100,
                "non-entity-ticking fangs must not decrement warmup at tick {tick}"
            );
        }

        // Fangs added during the current mob pass (id > ticked_entities_through) wait for next tick.
        game.server.ticked_entities_through = game.server.next_entity;
        game.add_fangs([6.5, 200.0, 3.5], 0.0, 100, 0);
        let late_id = game.fangs.list.last().unwrap().id;
        game.tick_mobs().unwrap();
        let late = game.fangs.list.iter().find(|f| f.id == late_id).unwrap();
        assert_eq!(
            motion_bits(late.motion),
            [0; 3],
            "fangs added during the mob pass must not be pushed in the same pass"
        );
        assert_eq!(
            late.warmup, 100,
            "late fangs must not tick in the same pass"
        );

        // Next full world tick admits them and pushes them to the exact tick-1 water_north bits.
        game.tick_world(true).unwrap();
        let late = game.fangs.list.iter().find(|f| f.id == late_id).unwrap();
        assert_eq!(late.warmup, 99);
        assert_eq!(
            motion_bits(late.motion),
            expected[&("water_north", 1)],
            "admitted fangs must be pushed on the next tick"
        );
    });

    // Fast-lava (Nether) cases against the exact 26.3 Nether witness.
    with_game(|game| {
        prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
        game.world.fast_lava = true;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.update_chunk_tickets();
        bind_local_player(game, 1, 99);
        game.set_allow_commands(true);
        game.build_registries();
        assert!(game.run_command("forceload add 0 0 15 15"));
        for command in [
            "fill 1 199 1 4 199 5 minecraft:stone",
            "fill 1 200 1 4 202 5 minecraft:barrier",
            "fill 1 200 1 4 202 5 minecraft:air",
            "setblock 2 200 4 minecraft:lava",
            "setblock 2 200 3 minecraft:lava[level=1]",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        let cases = [
            (
                "nether_fast_lava_north",
                [2362, 0, 0, 11],
                "summon minecraft:evoker_fangs 2.5 200 3.5 {UUID:[I;2362,0,0,11],Warmup:100}",
            ),
            (
                "nether_fast_lava_loaded",
                [2362, 0, 0, 12],
                "summon minecraft:evoker_fangs 2.5 200 3.5 {UUID:[I;2362,0,0,12],Warmup:100,Motion:[0.05d,0.0d,-0.02d]}",
            ),
        ];
        let mut spawned = Vec::with_capacity(cases.len());
        for (name, uuid, command) in cases {
            assert!(game.run_command(command), "{command}");
            let id = game.fangs.list.last().expect("summoned fangs").id;
            spawned.push((name, uuid, id));
        }
        for tick in 0..=2usize {
            if tick > 0 {
                game.tick_world(true).unwrap();
            }
            for &(name, uuid, id) in &spawned {
                let want = *expected
                    .get(&(name, tick))
                    .unwrap_or_else(|| panic!("missing witness row ({name}, {tick})"));
                check_fangs_motion(game, name, uuid, id, tick, want);
                checked += 1;
            }
        }
    });

    assert_eq!(checked, expected.len(), "every witness row must be checked");
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
