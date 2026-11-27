use sunrise_gameplay_common::game::Screen;

#[path = "local_rotation_tests_client.rs"]
pub(crate) mod local_rotation_tests;

pub(crate) fn connect(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> sunrise_protocol::connection::Pipe {
    let (client, server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "movement-client").unwrap();
    let remote = game.client.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.loaded_sent = true;
    remote.entity_id = 42;
    remote.level_load.finish();
    game.client.screen = Screen::Playing;
    game.client.streaming.insert_remote(
        &mut game.common.world,
        sunrise_level::chunk::Chunk::empty(0, 0),
    );
    server
}
