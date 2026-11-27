use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;

/// `Rabbit.getBreedOffspring`: the baby starts as a fresh biome variant
/// (`Rabbit.getRandomRabbitVariant`: the level's `nextInt(100)`), and unless
/// the breeding parent's `nextInt(20)` comes up 0, it takes a parent's colour
/// by that parent's `nextBoolean`. A brown and a white rabbit therefore give
/// white about 47.5% of the time, brown 47.5%, and a fresh biome variant 5%.
#[test]
#[ignore = "daily"]
fn a_baby_rabbit_takes_a_parents_colour_or_a_fresh_biome_variant() {
    use sunrise_core::random::{LegacyRandom, LevelRandom, RandomSource};
    let mut resources = None;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    let kind = game
        .res
        .mob_types()
        .get("minecraft:rabbit")
        .expect("a rabbit")
        .clone();
    // What a fresh variant can be here comes from the biome's tags.
    let facts = game.biome_facts(8, 64, 8);
    let fresh = |roll: i32| {
        if facts.white_rabbit {
            if roll < 80 { 1 } else { 3 }
        } else if facts.gold_rabbit {
            4
        } else if roll < 50 {
            0
        } else if roll < 90 {
            5
        } else {
            2
        }
    };
    let (part, mate_id) = (7u32, 8u32);
    let n = 2_000u32;
    let mut counts = [0usize; 8];
    let mut fresh_drawn = 0usize;
    for i in 0..n {
        let (parent_seed, level_seed) = (
            1_000_003 + 7_919 * i64::from(i),
            4_242 + 104_729 * i64::from(i),
        );
        game.world.random = LevelRandom::for_level(LegacyRandom::new(level_seed));
        game.mobs.clear();
        game.orbs.clear();
        // Ids the spawner hands out stay clear of the pair's.
        game.server.next_entity = 10_000 + 4 * i;
        for (id, variant) in [(part, 0u8), (mate_id, 1)] {
            let mut m = Mob::new(id, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0);
            m.random = LegacyRandom::new(parent_seed + i64::from(id));
            m.extra.variant = variant;
            m.extra.in_love = 600;
            game.mobs.push(m);
        }
        game.breed(
            part,
            mate_id,
            None,
            None,
            None,
            sunrise_sim::mobs::BreedExperience::Deferred,
            true,
        );
        let child = game
            .mobs
            .iter()
            .find(|m| m.id != part && m.id != mate_id)
            .expect("a baby rabbit");
        assert_eq!(child.extra.age, -24000, "{i}: a baby");
        // What vanilla's draws answer, in vanilla's order: the level's roll,
        // then the parent's one-in-twenty, then its coin toss.
        let mut level = LegacyRandom::new(level_seed);
        let biome_variant = fresh(level.next_int_bound(100).unwrap());
        let mut parent = LegacyRandom::new(parent_seed + i64::from(part));
        let expected = if parent.next_int_bound(20).unwrap() == 0 {
            fresh_drawn += 1;
            biome_variant
        } else if parent.next_boolean() {
            1
        } else {
            0
        };
        assert_eq!(
            child.extra.variant, expected,
            "{i}: the baby takes vanilla's draw"
        );
        // The parent's stream ends where vanilla's does: the coin, then
        // finalizeSpawnChildFromBreeding's 1-to-7 roll.
        if game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_DROPS)
        {
            let _ = parent.next_int_bound(7);
        }
        let mut drawn = game
            .mobs
            .iter()
            .find(|m| m.id == part)
            .expect("the parent")
            .random
            .clone();
        for _ in 0..4 {
            assert_eq!(
                drawn.next_int(),
                parent.next_int(),
                "{i}: the parent's stream goes on where vanilla leaves it"
            );
        }
        counts[usize::from(child.extra.variant)] += 1;
    }
    // The one-in-twenty biome case was reached, and the fresh variants that
    // differ from both parents showed up: 5% of the babies.
    assert!(fresh_drawn > 50, "one in twenty: {fresh_drawn} of {n}");
    let fresh_share = |v: u8| {
        if facts.white_rabbit {
            match v {
                1 => 0.04,
                3 => 0.01,
                _ => 0.0,
            }
        } else if facts.gold_rabbit {
            if v == 4 { 0.05 } else { 0.0 }
        } else {
            match v {
                0 => 0.025,
                5 => 0.02,
                2 => 0.005,
                _ => 0.0,
            }
        }
    };
    for v in 0..8u8 {
        let vanilla = fresh_share(v) + if v == 0 || v == 1 { 0.475 } else { 0.0 };
        let seen = counts[usize::from(v)] as f64 / f64::from(n);
        assert!(
            (seen - vanilla).abs() < 0.03,
            "variant {v}: {seen:.3} of the babies, vanilla gives {vanilla:.3}"
        );
    }
    assert!(
        counts[2] + counts[3] + counts[4] + counts[5] > 0,
        "a fresh biome variant: {counts:?}"
    );
}
