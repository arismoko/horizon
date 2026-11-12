//! `PlayerRef::respawn`: the Respawn button for a dead player.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_dead_player_is_respawned_once() {
    with_guests(|game| {
        let a = game.player(12).unwrap();
        assert!(!game.player_respawn(a), "alive");
        let i = game.guest_index(12).unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(i, |g| g.dead = true);
        // Not from inside a player's hook.
        game.as_guest(13, |game| assert!(!game.player_respawn(a)))
            .unwrap();
        assert!(game.player_respawn(a));
        assert!(!game.lan.as_ref().unwrap().hub.guest(12).unwrap().dead);
        assert!(!game.player_respawn(a), "alive again");
    });
}
