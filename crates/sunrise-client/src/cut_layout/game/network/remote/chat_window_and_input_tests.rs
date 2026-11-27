use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_protocol::connection::{Connection, Pipe};

/// `ChatComponent.addClientSystemMessage`: the line joins this client's
/// chat history and no packet leaves for the server.
#[test]
#[ignore = "daily"]
fn a_mod_chat_line_shows_locally_and_sends_nothing() {
    with_client_game(|game| {
        let (client, server) = Pipe::pair();
        game.connect_local(client, "ChatLineTest").unwrap();
        // Whatever connecting wrote is not this line's; drain it until the
        // client waits for the login answer, so the check below is about
        // `chat_line` alone.
        let (mut reader, _writer) = Connection::local(server).split();
        reader
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        while reader.recv().is_ok() {}
        mod_api::prelude::NetworkClientHost::chat_line(game, mod_api::Text::literal("local only"))
            .unwrap();
        let history: Vec<_> = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .chat
            .iter()
            .map(|(line, _)| line.clone())
            .collect();
        assert_eq!(history, ["local only"]);
        assert!(
            reader.recv().unwrap_err().is_timed_out(),
            "a local chat line sends nothing to the server"
        );
    });
}
