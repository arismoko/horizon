use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::ClientGame;
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

/// The calling mod's identity for `chat_line_tagged` (the mod whose hook is running).
fn in_mod(id: &'static str) -> mod_api::content::crash::ModGuard {
    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id,
        hook: "client_tick",
        part: None,
    })
}
fn plain_chat_line(game: &mut ClientGame<'_>, text: &str) {
    mod_api::prelude::NetworkClientHost::chat_line(game, mod_api::Text::literal(text)).unwrap();
}
fn tagged_chat_line(game: &mut ClientGame<'_>, tag: &str, text: &str) {
    mod_api::prelude::NetworkClientHost::chat_line_tagged(game, tag, mod_api::Text::literal(text))
        .unwrap();
}
fn removed_tagged(game: &mut ClientGame<'_>, tag: &str) -> usize {
    mod_api::prelude::NetworkClientHost::chat_remove_tagged(game, tag)
}
fn chat_lines(game: &ClientGame<'_>) -> Vec<String> {
    game.client.chat_ui.chat.recent_lines(100)
}
/// What the control socket counts as received: the connection's count, or the window's own chat's.
fn received_count(game: &ClientGame<'_>) -> u64 {
    game.client
        .remote
        .as_ref()
        .map_or(game.client.chat_ui.chat.received, |r| r.chat_received)
}

/// `chat_line_tagged` and `chat_remove_tagged` as a mod calls them (#6038): the owner is the
/// calling mod, so another mod's equal tag is another line; outside a mod's call the tag does
/// nothing (a plain `chat_line`, and nothing to remove); a replacement is counted as received like
/// a new line; and the Chat option's Hidden level does not hide the client's own line.
fn tagged_chat_round(game: &mut ClientGame<'_>, connected: bool) {
    plain_chat_line(game, "before");
    {
        let _call = in_mod("chat_tagged_a");
        tagged_chat_line(game, "status", "altar 10");
    }
    plain_chat_line(game, "after");
    assert_eq!(chat_lines(game), ["after", "altar 10", "before"]);
    // The same mod's second call replaces the line where it is, and counts as one more message.
    let count = received_count(game);
    {
        let _call = in_mod("chat_tagged_a");
        tagged_chat_line(game, "status", "altar 25");
    }
    assert_eq!(chat_lines(game), ["after", "altar 25", "before"]);
    assert_eq!(received_count(game), count + 1);
    if connected {
        let remote = game.client.remote.as_ref().unwrap();
        assert_eq!(remote.chat.last().unwrap().0, "altar 25");
    }
    // Another mod's equal tag is its own line, and its removal leaves the first mod's alone.
    {
        let _call = in_mod("chat_tagged_b");
        tagged_chat_line(game, "status", "other 1");
        assert_eq!(chat_lines(game)[0], "other 1");
        assert_eq!(removed_tagged(game, "status"), 1);
        assert_eq!(removed_tagged(game, "status"), 0);
    }
    assert_eq!(chat_lines(game), ["after", "altar 25", "before"]);
    {
        let _call = in_mod("chat_tagged_a");
        assert_eq!(removed_tagged(game, "status"), 1);
    }
    assert_eq!(chat_lines(game), ["after", "before"]);
    // Outside a mod's call the tag is ignored: each call adds a plain line, and none is removed.
    assert_eq!(removed_tagged(game, "status"), 0);
    tagged_chat_line(game, "status", "plain 1");
    tagged_chat_line(game, "status", "plain 2");
    assert_eq!(&chat_lines(game)[..2], ["plain 2", "plain 1"]);
    {
        let _call = in_mod("chat_tagged_a");
        tagged_chat_line(game, "status", "mine");
        assert_eq!(removed_tagged(game, "status"), 1);
    }
    assert_eq!(&chat_lines(game)[..2], ["plain 2", "plain 1"]);
    // The Chat option's Hidden level keeps the client's own lines (`ChatAbilities::shows`), so the
    // line is added, keyed, and replaced as at any other level.
    game.client.chat_ui.chat.set_abilities(
        &game.client.gui.atlas.font,
        sunrise_client::chat::ChatAbilities::new(sunrise_client::chat::ChatVisibility::Hidden),
    );
    let _call = in_mod("chat_tagged_a");
    tagged_chat_line(game, "hidden", "h1");
    assert_eq!(chat_lines(game)[0], "h1");
    let len = chat_lines(game).len();
    tagged_chat_line(game, "hidden", "h2");
    assert_eq!(chat_lines(game)[0], "h2");
    assert_eq!(chat_lines(game).len(), len);
}

#[test]
#[ignore = "daily"]
fn a_mods_tagged_chat_line_replaces_and_removes_on_the_windows_own_chat() {
    with_client_game(|game| {
        assert!(game.client.remote.is_none());
        tagged_chat_round(game, false);
    });
}

#[test]
#[ignore = "daily"]
fn a_mods_tagged_chat_line_replaces_and_removes_on_a_connections_chat() {
    with_client_game(|game| {
        let (client, server) = Pipe::pair();
        game.connect_local(client, "TaggedChatTest").unwrap();
        let (_reader, _writer) = Connection::local(server).split();
        tagged_chat_round(game, true);
    });
}
