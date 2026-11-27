#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::server_side::dragon::tests::flying;

    use sunrise_core::random::LegacyRandom;
    use sunrise_gameplay_common::game::dragon_common::Phase;

    use crate::game::client_side::dragon_client::breath_puffs;

    /// `DragonLandingPhase.doClientTick`: eight puffs a tick from the head,
    /// along `getHeadLookVector(1)` (a quarter turn of it is thrown away in
    /// vanilla) plus the dragon's own motion.
    #[test]
    fn a_landing_dragon_breathes_eight_puffs_from_its_head() {
        let look = [0.0, -0.6, 0.8];
        let head = [10.0, 70.5, -3.0];
        let motion = [0.25, -0.05, 0.5];
        let mut random = LegacyRandom::new(7);
        let puffs = breath_puffs(
            &Phase::Landing { target: None },
            look,
            head,
            motion,
            &mut random,
        );
        assert_eq!(puffs.len(), 8);
        let speed = [
            -look[0] * f64::from(0.08_f32) + motion[0],
            -look[1] * f64::from(0.3_f32) + motion[1],
            -look[2] * f64::from(0.08_f32) + motion[2],
        ];
        for (at, v) in puffs {
            assert_eq!(v, speed);
            // The head's centre, jittered by `nextGaussian() / 2`.
            for a in 0..3 {
                assert!(
                    (at[a] - head[a]).abs() < 2.0,
                    "puff {} off the head: {at:?}",
                    a
                );
            }
        }
    }
    /// `DragonSittingFlamingPhase.doClientTick`: on the even ticks of the first
    /// ten, eight starts each breathe a fan of six puffs down the look line.
    #[test]
    fn a_flaming_dragon_breathes_a_fan_on_every_second_tick() {
        let look = [0.6, -0.0, 0.8];
        let head = [0.0, 65.5, 4.0];
        for (tick, want) in [(1, 0), (2, 48), (3, 0), (4, 48), (8, 48), (9, 0), (10, 0)] {
            let mut random = LegacyRandom::new(11);
            let puffs = breath_puffs(
                &Phase::Flaming { ticks: tick },
                look,
                head,
                [0.0; 3],
                &mut random,
            );
            assert_eq!(puffs.len(), want, "flame tick {tick}");
        }
        let mut random = LegacyRandom::new(11);
        let puffs = breath_puffs(
            &Phase::Flaming { ticks: 2 },
            look,
            head,
            [0.0; 3],
            &mut random,
        );
        for (i, (at, v)) in puffs.iter().enumerate() {
            let j = (i % 6) as f64;
            // Six puffs leave from each of the eight starts.
            assert_eq!(at, &puffs[i - i % 6].0);
            assert_eq!(
                *v,
                [
                    -look[0] * f64::from(0.08_f32) * j,
                    -look[1] * f64::from(0.6_f32),
                    -look[2] * f64::from(0.08_f32) * j,
                ]
            );
        }
    }
    /// No other phase has a `doClientTick` that breathes.
    #[test]
    fn only_the_landing_and_flaming_phases_breathe() {
        let mut random = LegacyRandom::new(3);
        for phase in [
            flying(),
            Phase::Scanning { time: 5 },
            Phase::Hover { target: None },
            Phase::Takeoff {
                first: false,
                path: None,
                target: None,
            },
            Phase::Attacking { ticks: 3 },
            Phase::Dying {
                target: None,
                time: 0,
            },
        ] {
            assert!(
                breath_puffs(&phase, [0.0, -1.0, 0.0], [0.0; 3], [0.0; 3], &mut random).is_empty(),
                "{phase:?}"
            );
        }
    }
}
