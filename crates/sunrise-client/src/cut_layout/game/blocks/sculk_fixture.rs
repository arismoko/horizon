#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    #[test]
    #[ignore = "daily"]
    fn fixture_death_outside_catalyst_range_drops_experience() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            let input = scratch.path().join("scenarios.txt");
            let output = scratch.path().join("result.txt");
            std::fs::write(
                &input,
                concat!(
                    "scenario death ticks=3\n",
                    "block 0 -60 0 minecraft:sculk_catalyst\n",
                    "player 5.5 -60 5.5\n",
                    "mob minecraft:silverfish 9.5 -60 0.5 0 7 noai\n",
                    "at 3 kill m0\nend\n",
                ),
            )
            .unwrap();
            game.sculk_fixture(&format!("{},{}", input.display(), output.display()))
                .unwrap();
            assert_eq!(
                game.orbs
                    .iter()
                    .map(|orb| i64::from(orb.value) * i64::from(orb.count))
                    .sum::<i64>(),
                5
            );
        });
    }
}
