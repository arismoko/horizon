//! `PlayerRef::kick`: the player is disconnected with the mod's reason.
use super::native_guest_turn_tests::with_guests;
use horizon_client::text::Text;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_kick_disconnects_the_player_with_its_reason() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        assert!(game.player_kick(b, Text::literal("Too far out")));
        let host = game.lan.as_ref().unwrap();
        assert_eq!(host.hub.guest(13).unwrap().left_reason, "Too far out");
        assert!(host.hub.guest(12).unwrap().left_reason.is_empty());
        // Inside the player's own turn, as a hook kicks the player it hears.
        let a = game.player(12).unwrap();
        game.as_guest(12, |game| assert!(game.player_kick(a, Text::literal("Bye"))))
            .unwrap();
        assert_eq!(game.lan.as_ref().unwrap().hub.guest(12).unwrap().left_reason, "Bye");
    });
}
