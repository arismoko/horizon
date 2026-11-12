//! A skin change: the profile property is set, and the player is sent to the others again.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_new_skin_is_set_and_the_player_sent_again() {
    with_guests(|game| {
        let a = game.player(12).unwrap();
        assert!(game.player_set_profile_property(a, "textures", "old", None));
        assert!(game.player_set_profile_property(a, "textures", "new", Some("signed")));
        assert!(!game.player_set_profile_property(a, "", "value", None));
        assert!(!game.player_set_profile_property(a, "textures", &"x".repeat(32768), None));
        let properties = |game: &super::Game<'_>| {
            let g = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
            g.properties
                .iter()
                .map(|p| (p.name.clone(), p.value.clone(), p.signature.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            properties(game),
            [("textures".into(), "new".into(), Some("signed".into()))]
        );
        // Hidden from everyone for a whole tick, then shown again.
        assert!(game.player_refresh(a));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        assert_eq!(hub.hidden.get(&12), Some(&u8::MAX));
        assert!(!hub.sees(4, 12));
        hub.ticks += 1;
        hub.end_refreshes();
        assert_eq!(hub.hidden.get(&12), Some(&u8::MAX));
        hub.ticks += 1;
        hub.end_refreshes();
        assert_eq!(hub.hidden.get(&12), None);
        // A player a mod hid stays hidden at that level afterwards.
        hub.hidden.insert(13, 3);
        let b = game.player(13).unwrap();
        assert!(game.player_refresh(b));
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.ticks += 2;
        hub.end_refreshes();
        assert_eq!(hub.hidden.get(&13), Some(&3));
    });
}
