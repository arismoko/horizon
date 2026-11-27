#![cfg(test)]
#[cfg(test)]
pub(crate) mod audience_tests {

    use sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake;

    #[test]
    #[ignore = "daily"]
    fn fake_players_are_not_boss_viewers() {
        super::super::native_entity_refs::tests::with_game(|game| {
            add_real_and_fake(game);
            let viewers = game.boss_viewers();
            assert!(
                viewers
                    .iter()
                    .any(|v| v.uuid == 1 && v.here && v.at == [0.0, 100.0, 0.0])
            );
            assert!(!viewers.iter().any(|v| v.uuid == 2));
        });
    }
}

#[cfg(test)]
#[path = "tests/bossbar_names_tests.rs"]
pub(crate) mod bossbar_names_tests;

#[cfg(test)]
#[path = "tests/bossbar_store_tests.rs"]
pub(crate) mod bossbar_store_tests;
