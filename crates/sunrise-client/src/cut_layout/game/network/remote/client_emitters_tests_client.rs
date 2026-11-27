use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_level::particle_options::ParticleType::{self};

// Capture the level requests, before the limiter or providers consume random
// draws. The expected sequence uses the 26.3 TrackingEmitter sampling contract
// and independent feet/dimensions, never the packet handler's body lookup.
pub(in crate::game) fn assert_bursts(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    random: &mut LegacyRandom,
    bodies: &[(ParticleType, [f64; 3], f32, f32)],
) {
    let mut expected = Vec::new();
    for &(ty, [x, y, z], width, height) in bodies {
        for _ in 0..16 {
            let [dx, dy, dz] = std::array::from_fn(|_| f64::from(random.next_float()) * 2.0 - 1.0);
            if dx * dx + dy * dy + dz * dz <= 1.0 {
                let at = [
                    x + f64::from(width) * (0.25 * dx),
                    y + f64::from(height) * (0.5 + 0.25 * dy),
                    z + f64::from(width) * (0.25 * dz),
                ];
                expected.push((
                    ty,
                    at.map(f64::to_bits),
                    [dx, dy + 0.2, dz].map(f64::to_bits),
                ));
            }
        }
    }
    let actual: Vec<_> = game
        .client
        .particles
        .0
        .requested
        .as_mut()
        .unwrap()
        .drain(..)
        .map(|(options, at, motion)| (options.ty, at.map(f64::to_bits), motion.map(f64::to_bits)))
        .collect();
    assert_eq!(actual, expected);
}

pub(in crate::game) fn capture_requests(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> LegacyRandom {
    const SEED: i64 = 719;
    game.client.particles.0.seed_randoms(SEED, 11, 12);
    game.client.particles.0.requested = Some(Vec::new());
    LegacyRandom::new(SEED)
}
