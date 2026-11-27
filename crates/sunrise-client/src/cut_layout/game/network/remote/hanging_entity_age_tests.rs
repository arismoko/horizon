use crate::game::server_side::native_entity_refs::tests::with_client_game;

/// A client's painting lives on `Entity.tickCount`, which its renderer reads as the age: it
/// advances with each running level tick and holds while the level is frozen.
#[test]
#[ignore = "daily"]
fn a_client_painting_ages_with_running_ticks() {
    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "painting-age-client").unwrap();
        assert!(game.spawn_remote_object(5, "minecraft:painting", [0.5, 64.5, 0.5], [0.0; 3], 2));
        assert_eq!(game.paintings[0].ticks, 0);
        game.tick_remote_entities(true);
        game.tick_remote_entities(true);
        assert_eq!(game.paintings[0].ticks, 2);
        game.tick_remote_entities(false);
        assert_eq!(game.paintings[0].ticks, 2);
    });
}

/// A client's item frame lives on `Entity.tickCount`, which its renderer reads as the age: it
/// advances with each running level tick and holds while the level is frozen.
#[test]
#[ignore = "daily"]
fn a_client_item_frame_ages_with_running_ticks() {
    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "item-frame-age-client").unwrap();
        assert!(game.spawn_remote_object(6, "minecraft:item_frame", [0.5, 64.5, 0.5], [0.0; 3], 2));
        assert_eq!(game.item_frames[0].ticks, 0);
        game.tick_remote_entities(true);
        game.tick_remote_entities(true);
        assert_eq!(game.item_frames[0].ticks, 2);
        game.tick_remote_entities(false);
        assert_eq!(game.item_frames[0].ticks, 2);
    });
}
