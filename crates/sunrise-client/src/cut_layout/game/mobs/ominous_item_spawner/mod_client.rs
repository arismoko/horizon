#![cfg(test)]
#[cfg(test)]
mod client_clock_tests {
    #[test]
    #[ignore = "daily"]
    fn client_spawner_clock_advances() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            game.add_remote_ominous_item_spawner(701, [1.0, 80.0, 1.0]);
            game.tick_remote_ominous_item_spawners();
            assert_eq!(game.ominous_spawners[0].tick_count, 1);
        });
    }
}
