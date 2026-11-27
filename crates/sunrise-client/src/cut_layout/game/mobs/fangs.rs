#![cfg(test)]
#[cfg(test)]
#[path = "fangs/current_tests.rs"]
pub(crate) mod current_tests;

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(feature = "client")]
    use sunrise_client_gameplay::game::client_side::fangs_client::fang_crit;
    use sunrise_core::random::LegacyRandom;
    use sunrise_gameplay_common::game::fangs_common::fang_attack_pitch;
    use {
        sunrise_gameplay::game::server_side::fangs::Party,
        sunrise_gameplay::game::server_side::fangs::fangs_allied,
    };

    #[test]
    #[ignore = "daily"]
    fn taken_fangs_tick_keeps_survivors_before_both_spawns() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.fangs.list.clear();
            game.add_fangs([0.0, 80.0, 0.0], 0.0, 30, 0);
            game.add_fangs([1.0, 80.0, 0.0], 0.0, 30, 0);
            let first = game.fangs.list[0].id;
            let second = game.fangs.list[1].id;
            let next = game.server.next_entity;
            crate::game::server_side::entity_server::taken_pass_tests::with_spawn(
                |game| {
                    assert!(game.fangs.list.is_empty());
                    game.add_fangs([2.0, 80.0, 0.0], 0.0, 40, 0);
                    game.add_fangs([3.0, 80.0, 0.0], 0.0, 50, 0);
                },
                || game.tick_fangs(),
            );
            assert_eq!(
                game.fangs
                    .list
                    .iter()
                    .map(|f| (f.id, f.warmup))
                    .collect::<Vec<_>>(),
                [(first, 29), (second, 29), (next + 1, 40), (next + 2, 50)]
            );
        });
    }

    #[test]
    fn fang_attack_pitch_matches_the_vanilla_span() {
        // Java Random seed 1, first nextFloat() * 0.2 + 0.85, in f32.
        let mut random = LegacyRandom::new(1);
        let pitch = fang_attack_pitch(&mut random);
        assert!((pitch - 0.996_175_65).abs() < 1.0e-6, "{pitch}");
        assert!((0.85..1.05).contains(&pitch));
    }

    #[cfg(feature = "client")]
    #[test]
    fn fang_crit_sits_above_the_jaws() {
        // Java Random seed 1, the first crit of the twelve. The y is
        // getY() + 0.05 + nextDouble(), then addParticle adds 1.
        let mut random = LegacyRandom::new(1);
        let (at, velocity) = fang_crit(&mut random, [2.0, 70.0, -4.0]);
        let expected_at = [
            2.115_439_095_351_645_5,
            71.460_080_811_492_2,
            -4.146_142_579_345_142,
        ];
        let expected_velocity = [
            -0.100_369_766_424_293_3,
            0.590_326_772_827_236_1,
            -0.296_329_690_640_543_2,
        ];
        for i in 0..3 {
            assert!((at[i] - expected_at[i]).abs() < 1.0e-12, "{at:?}");
            assert!(
                (velocity[i] - expected_velocity[i]).abs() < 1.0e-12,
                "{velocity:?}"
            );
        }
    }

    fn party(illager: bool, team: Option<&sunrise_game_data::scoreboard::Team>) -> Party<'_> {
        Party { illager, team }
    }

    #[test]
    fn fangs_allies_follow_teams_and_the_illager_tag() {
        let mut board = sunrise_game_data::scoreboard::Scoreboard::default();
        board.add_team("red");
        board.add_team("blue");
        let red = board.team("red").unwrap();
        let blue = board.team("blue").unwrap();
        let evoker = |t| party(true, t);
        // Teamless illagers are friends.
        assert!(fangs_allied(evoker(None), party(true, None), None));
        // Different teams: the illager tag no longer applies.
        assert!(!fangs_allied(
            evoker(Some(red)),
            party(true, Some(blue)),
            None
        ));
        // One side with a team: still not allied.
        assert!(!fangs_allied(evoker(Some(red)), party(true, None), None));
        // The same team allies a non-illager mob.
        assert!(fangs_allied(
            evoker(Some(red)),
            party(false, Some(red)),
            None
        ));
        assert!(!fangs_allied(evoker(None), party(false, None), None));
        // A vex follows its root owner.
        assert!(fangs_allied(
            evoker(None),
            party(false, None),
            Some((true, evoker(None)))
        ));
        assert!(!fangs_allied(
            evoker(Some(red)),
            party(false, None),
            Some((false, party(true, Some(blue))))
        ));
    }
}

#[cfg(test)]
#[path = "fangs/ticking_tests.rs"]
pub(crate) mod ticking_tests;
