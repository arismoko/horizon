//! `server_max_players` / `server_set_max_players`: the live login limit.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_reads_and_sets_the_player_limit() {
    with_guests(|game| {
        let before = game.server_max_players().unwrap();
        assert!(game.server_set_max_players(before + 40));
        assert_eq!(game.server_max_players(), Some(before + 40));
        let shared = &game.lan.as_ref().unwrap().hub.lan.shared;
        assert_eq!(shared.max_players(), before as usize + 40);
    });
}
