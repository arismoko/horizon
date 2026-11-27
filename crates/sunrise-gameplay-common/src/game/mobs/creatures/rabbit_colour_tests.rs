/// `Rabbit.getRandomRabbitVariant`: the biome's tags pick the split, and the
/// `nextInt(100)` is drawn even where a tag decides alone (the gold rabbit),
/// so a spawn's and a bred baby's level random move on by the same one draw.
#[test]
fn the_biome_tags_pick_the_rabbit_variant() {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    let mut white = [0usize; 8];
    let mut temperate = [0usize; 8];
    for seed in 0..2_000i64 {
        let mut r = LegacyRandom::new(seed);
        white[usize::from(sunrise_sim::mobs::rabbit::random_variant(
            true, false, &mut r,
        ))] += 1;
        let mut r = LegacyRandom::new(seed);
        temperate[usize::from(sunrise_sim::mobs::rabbit::random_variant(
            false, false, &mut r,
        ))] += 1;
    }
    assert_eq!(
        white[1] + white[3],
        2_000,
        "a white rabbit biome: {white:?}"
    );
    assert!(
        (1_550..1_650).contains(&white[1]),
        "80% white, 20% splotched: {white:?}"
    );
    assert_eq!(
        temperate[0] + temperate[2] + temperate[5],
        2_000,
        "elsewhere: {temperate:?}"
    );
    assert!(
        (900..1_100).contains(&temperate[0])
            && (700..900).contains(&temperate[5])
            && (100..300).contains(&temperate[2]),
        "50/40/10 brown, salt, black: {temperate:?}"
    );
    // The gold rabbit's tag decides alone, but its roll is still drawn.
    let mut a = LegacyRandom::new(7);
    assert_eq!(
        sunrise_sim::mobs::rabbit::random_variant(false, true, &mut a),
        4
    );
    let mut b = LegacyRandom::new(7);
    let _ = b.next_int_bound(100);
    assert_eq!(a.next_int(), b.next_int(), "the roll is drawn for gold");
}
