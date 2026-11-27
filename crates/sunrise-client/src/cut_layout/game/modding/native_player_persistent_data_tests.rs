//! Scoped persistent-data reads and key edits against real LAN guests, through player handles.

use mod_api::{Compound, JavaString, PlayerPersistentDataError, Tag, prelude::*};
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;

fn saved_data<'a>(
    game: &'a sunrise_gameplay::game::ServerGame<'_>,
    conn: u32,
) -> Option<&'a Compound> {
    let hub = &game.server.lan.as_ref().unwrap().hub;
    match hub.guest(conn)?.saved.as_ref()?.get("NeoForgeData")? {
        Tag::Compound(data) => Some(data),
        _ => None,
    }
}

/// A chain of `levels` compounds, each holding the next under `k`.
fn nested(levels: usize) -> Tag {
    let mut tag = Tag::Compound(Compound::new());
    for _ in 1..levels {
        tag = Tag::compound([("k", tag)]);
    }
    tag
}

#[test]
#[ignore = "daily"]
fn persistent_data_is_kept_in_the_players_record() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let key = JavaString::from("husksync:key");
        let other = JavaString::from("husksync:other");
        assert_eq!(b.persistent_data(game), None);
        b.set_persistent_data_key(game, &key, None).unwrap();
        assert_eq!(saved_data(game, 13), None);

        b.set_persistent_data_key(game, &key, Some(&Tag::Int(7)))
            .unwrap();
        b.set_persistent_data_key(game, &other, Some(&Tag::Int(9)))
            .unwrap();
        let data = b.persistent_data(game).unwrap();
        assert_eq!(data.get(&key), Some(&Tag::Int(7)));
        assert!(std::ptr::eq(data, saved_data(game, 13).unwrap()));
        assert_eq!(saved_data(game, 12), None);

        game.as_guest(12, |game| {
            assert_eq!(
                b.persistent_data(game).unwrap().get(&other),
                Some(&Tag::Int(9))
            );
            a.set_persistent_data_key(game, &key, Some(&Tag::Int(3)))
                .unwrap();
            b.set_persistent_data_key(game, &key, None).unwrap();
            let Some(Tag::Compound(resident)) = game
                .level
                .player_extra
                .as_ref()
                .unwrap()
                .get("NeoForgeData")
            else {
                panic!("acting player's compound did not reach the save record");
            };
            assert!(std::ptr::eq(a.persistent_data(game).unwrap(), resident));
        })
        .unwrap();
        assert_eq!(saved_data(game, 12).unwrap().get(&key), Some(&Tag::Int(3)));
        assert_eq!(
            saved_data(game, 13),
            Some(&Compound::from([(other.clone(), Tag::Int(9))]))
        );
        b.set_persistent_data_key(game, &other, None).unwrap();
        assert_eq!(saved_data(game, 13), Some(&Compound::new()));
        assert_eq!(b.persistent_data(game), Some(&Compound::new()));
    });
}

/// The written entry is two levels below the root, so its last compound may be at 511,
/// but not 512. Dead online players still accept persistent data edits (rolling back on
/// transaction abort and carrying `PlayerPersisted` across respawn); stale and client
/// refusals write nothing.
#[test]
#[ignore = "daily"]
fn each_persistent_data_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        use PlayerPersistentDataError::{InvalidNbt, Stale, TooDeep};
        let b = game.players().server_players()[1];
        let key = JavaString::from("kept");
        b.set_persistent_data_key(game, &key, Some(&nested(510)))
            .unwrap();
        assert_eq!(
            b.persistent_data(game).unwrap().get(&key),
            Some(&nested(510))
        );
        b.set_persistent_data_key(game, &key, Some(&Tag::Byte(1)))
            .unwrap();
        assert_eq!(
            b.set_persistent_data_key(game, &key, Some(&nested(511))),
            Err(TooDeep)
        );
        // NUL costs two modified UTF-8 bytes per UTF-16 unit: 65,536 bytes do not fit.
        let too_long = JavaString(vec![0; 32_768]);
        assert_eq!(
            b.set_persistent_data_key(game, &too_long, Some(&Tag::Byte(2))),
            Err(InvalidNbt)
        );
        assert_eq!(
            b.set_persistent_data_key(game, &key, Some(&Tag::End)),
            Err(InvalidNbt)
        );
        let kept = Compound::from([(key.clone(), Tag::Byte(1))]);
        assert_eq!(b.persistent_data(game), Some(&kept));

        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        let persisted = JavaString::from("PlayerPersisted");
        let persisted_value = Tag::compound([("dead_edit", Tag::Int(42))]);
        let _: Result<(), ()> = game.storage_transaction(|host, _| {
            b.set_persistent_data_key(host, &key, None).unwrap();
            b.set_persistent_data_key(host, &persisted, Some(&persisted_value))
                .unwrap();
            assert_eq!(
                b.persistent_data(host).unwrap().get(&persisted),
                Some(&persisted_value)
            );
            assert_eq!(b.persistent_data(host).unwrap().get(&key), None);
            Err(())
        });
        assert_eq!(
            b.persistent_data(game),
            Some(&kept),
            "aborting a transaction restores a dead player's prior persistent data"
        );
        b.set_persistent_data_key(game, &persisted, Some(&persisted_value))
            .unwrap();
        assert_eq!(
            b.persistent_data(game).unwrap().get(&persisted),
            Some(&persisted_value)
        );
        game.guest_respawn(13);
        let after_respawn = b.persistent_data(game).unwrap();
        assert_eq!(after_respawn.get(&persisted), Some(&persisted_value));
        assert_eq!(after_respawn.get(&key), None);

        let index = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .connection_index(13)
            .unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.remove(index);
        assert_eq!(b.set_persistent_data_key(game, &key, None), Err(Stale));
        assert_eq!(b.persistent_data(game), None);
        // Client refusal is enforced at compile time (`ClientGame` lacks server player persistent-data ops).
    });
}

#[test]
#[ignore = "daily"]
fn a_respawn_keeps_only_player_persisted() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let outside = JavaString::from("outside");
        let persisted = JavaString::from("PlayerPersisted");
        let inside = Tag::compound([("inside", Tag::Int(2))]);
        b.set_persistent_data_key(game, &outside, Some(&Tag::Int(1)))
            .unwrap();
        b.set_persistent_data_key(game, &persisted, Some(&inside))
            .unwrap();
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        game.guest_respawn(13);
        let after = b.persistent_data(game).unwrap();
        assert_eq!(after.get(&persisted), Some(&inside));
        assert_eq!(after.get(&outside), None);
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
