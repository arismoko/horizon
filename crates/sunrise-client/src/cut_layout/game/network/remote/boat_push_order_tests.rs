#![cfg(feature = "client")]
use sunrise_gameplay_common::game::remote::Vec3;
use sunrise_sim::motion::PlayerMotion;

/// ClientLevel ticks entities in the order they joined: a boat added after the
/// player pushes it after its move, one the level held when the player respawned
/// pushes it before.
#[test]
#[ignore = "daily"]
fn a_boat_pushes_the_player_in_the_level_tick_order() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        let (_peer, _writer) = super::tests::connect_menu_test(game);
        let mut player = PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.7)).unwrap();
        player.client_side = true;
        game.player = Some(player);
        game.mobs.clear();
        let boat = sunrise_gameplay_common::game::boats_common::Boat::new(
            9,
            "oak".into(),
            Vec3::new(0.5, 64.0, 3.5),
            0.0,
        );
        game.boats.push(boat);
        let after = game.remote_pushes(false);
        assert_eq!(game.remote_pushes(true), Vec3::ZERO);
        assert!(after.z > 0.0, "{after:?}");
        game.client
            .remote
            .as_mut()
            .unwrap()
            .ticked_before_player
            .insert(9);
        assert_eq!(game.remote_pushes(false), Vec3::ZERO);
        assert_eq!(game.remote_pushes(true), after);
    });
}
