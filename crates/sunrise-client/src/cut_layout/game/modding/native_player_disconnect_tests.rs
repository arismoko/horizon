//! `player_disconnect` against real LAN guests: the reason reaches the right one only.

use mod_api::{Text, prelude::*};
use std::sync::{Arc, mpsc::Receiver};
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_protocol::text::translatable;
use sunrise_sim::lan::{Out, PlayerSender};

fn left_reason(game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> String {
    let hub = &game.server.lan.as_ref().unwrap().hub;
    hub.guest(conn).unwrap().left_reason.clone()
}

/// Gives guest `conn` a test channel, to read what its connection is sent.
fn listen(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> Receiver<Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.update_guest(conn, |g| g.sender = PlayerSender::new(tx, Arc::default()));
    rx
}

/// The disconnect reasons a guest's connection was sent.
fn disconnects(rx: &Receiver<Out>) -> Vec<Tag> {
    rx.try_iter()
        .filter_map(|out| match out {
            Out::Disconnect(reason) => Some(reason),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn disconnect_sends_the_reason_to_that_guest_only() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let (to_a, to_b) = (listen(game, 12), listen(game, 13));
        assert!(
            a.disconnect(game, Text::literal("Server restarting"))
                .unwrap()
        );
        assert_eq!(left_reason(game, 12), "Server restarting");
        assert_eq!(disconnects(&to_a).len(), 1);
        assert_eq!(left_reason(game, 13), "");
        assert!(disconnects(&to_b).is_empty());
        assert!(
            b.disconnect(
                game,
                Text::translate("multiplayer.disconnect.server_shutdown", vec![])
            )
            .unwrap()
        );
        // The client is sent the key to translate; with no server language the log gives it.
        assert_eq!(
            disconnects(&to_b),
            [translatable("multiplayer.disconnect.server_shutdown", &[])]
        );
        assert_eq!(
            left_reason(game, 13),
            "multiplayer.disconnect.server_shutdown"
        );
        assert_eq!(left_reason(game, 12), "Server restarting");
        assert!(disconnects(&to_a).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn disconnect_reaches_acting_and_displaced_players() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let (to_a, to_b) = (listen(game, 12), listen(game, 13));
        // In guest12's turn, guest13 is displaced: their record is reached, not their body.
        let kicked = game
            .as_guest(12, |game| {
                assert_eq!(game.players().acting_player(), Some(a));
                b.disconnect(game, Text::literal("Displaced by guest12"))
                    .unwrap()
            })
            .unwrap();
        assert!(kicked);
        assert_eq!(left_reason(game, 13), "Displaced by guest12");
        assert_eq!(disconnects(&to_b).len(), 1);
        assert_eq!(left_reason(game, 12), "");
        // The acting player, inside their own turn.
        let kicked = game
            .as_guest(12, |game| {
                a.disconnect(game, Text::literal("Bye from your own hook"))
                    .unwrap()
            })
            .unwrap();
        assert!(kicked);
        assert_eq!(left_reason(game, 12), "Bye from your own hook");
        assert_eq!(disconnects(&to_a).len(), 1);
    });
}

/// A disconnect is pending until the connection is removed: a second call answers false,
/// sends nothing and keeps the first reason.
#[test]
#[ignore = "daily"]
fn a_second_disconnect_while_pending_is_refused() {
    with_guests(|game| {
        let a = game.players().server_players()[0];
        let to_a = listen(game, 12);
        assert!(a.disconnect(game, Text::literal("First reason")).unwrap());
        assert!(!a.disconnect(game, Text::literal("Second reason")).unwrap());
        let refused = game
            .as_guest(12, |game| {
                a.disconnect(game, Text::literal("Third reason")).unwrap()
            })
            .unwrap();
        assert!(!refused);
        assert_eq!(left_reason(game, 12), "First reason");
        let hub = &game.server.lan.as_ref().unwrap().hub;
        assert!(hub.guest(12).unwrap().disconnect_pending);
        assert!(!hub.guest(13).unwrap().disconnect_pending);
        let sent = disconnects(&to_a);
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sunrise_protocol::text::plain(&sent[0], &Default::default()),
            "First reason"
        );
    });
}

#[test]
#[ignore = "daily"]
fn disconnect_refuses_the_owner_stale_and_offline_players() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        game.server.lan.as_mut().unwrap().hub.singleplayer_owner = Some(12);
        assert!(!a.disconnect(game, Text::literal("owner")).unwrap());
        game.server.lan.as_mut().unwrap().hub.singleplayer_owner = None;

        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                a.entity().id(),
            ),
            a.uuid(),
        );
        let wrong_uuid = mod_api::__private::game::player_ref(a.entity(), 99);
        let wrong_entity = mod_api::__private::game::player_ref(b.entity(), a.uuid());
        for invalid in [stale, wrong_uuid, wrong_entity] {
            assert!(!invalid.disconnect(game, Text::literal("stale")).unwrap());
        }

        let mut too_deep = Text::literal("leaf");
        for _ in 1..513 {
            too_deep = Text::literal("parent").append(too_deep);
        }
        assert!(matches!(
            a.disconnect(game, too_deep),
            Err(mod_api::text::TextFormError::TooDeep)
        ));
        assert_eq!(left_reason(game, 12), "");
        assert_eq!(left_reason(game, 13), "");

        game.server.lan.as_mut().unwrap().hub.guests.clear();
        assert!(!a.disconnect(game, Text::literal("offline")).unwrap());
    });
}
