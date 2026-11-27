//! The tracking hooks at their pairing (#4522, NeoForge's `PlayerEvent.StartTracking` and
//! `StopTracking` at the end of `ServerEntity.addPairing` and `removePairing`): a start is heard
//! with the entity in the level once the player was sent its spawn bundle, a stop once it was
//! sent the removal, and a payload a listener sends reaches the player right after that packet.
//! A leaving player's stops follow `player_logged_out`.
use mod_api::{Hooks, ModEntry, PayloadBytes, PayloadId, Registrar, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{buf::Reader, ids::play::clientbound as id};
use sunrise_sim::{
    lan::{PlayerSender, hub::HostEvent},
    raw_payloads::{Side, Transport},
};
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

pub(crate) const CHANNEL: &str = "tracktest:seen";

#[derive(Debug, PartialEq)]
pub(crate) enum Heard {
    /// A start, and whether the entity was in the level then.
    Start(u32, bool),
    Stop(u32),
    LoggedOut,
}

pub(crate) struct Probe(pub(crate) Arc<Mutex<Vec<Heard>>>);
impl Probe {
    /// Tell the player which entity it heard of, on the mod's channel.
    pub(crate) fn send(&self, host: &mut dyn ServerHost, player: u128, what: &str, entity: u32) {
        let Some(player) = host.player(player) else {
            return;
        };
        let body = format!("{what} {entity}").into_bytes();
        player
            .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(body))
            .unwrap();
    }
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_raw(PayloadId(CHANNEL.into()), None);
    }
    fn player_start_tracking(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerStartTrackingEvent<'_>,
    ) {
        let there = host.entity(event.entity).is_some();
        self.0
            .lock()
            .unwrap()
            .push(Heard::Start(event.entity, there));
        self.send(host, event.player, "start", event.entity);
    }
    fn player_stop_tracking(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerStopTrackingEvent<'_>,
    ) {
        self.0.lock().unwrap().push(Heard::Stop(event.entity));
        self.send(host, event.player, "stop", event.entity);
    }
    fn player_logged_out(&self, _: &mut dyn ServerHost, _: &mut mod_api::PlayerLoggedOutEvent<'_>) {
        self.0.lock().unwrap().push(Heard::LoggedOut);
    }
}

/// What the player was sent, read back from its connection: `Add(id)` for an entity's
/// `AddEntity`, `End` for a bundle delimiter, `Remove(ids)`, `Said(text)` for the mod's payload,
/// and `Other` with the packet id for the rest.
#[derive(Debug, PartialEq)]
pub(crate) enum Sent {
    Add(u32),
    End,
    Remove(Vec<u32>),
    Said(String),
    Other(i32),
}

pub(crate) fn read(packets: Vec<Vec<u8>>) -> Vec<Sent> {
    packets
        .iter()
        .map(|body| {
            let mut r = Reader::new(body);
            match r.varint().unwrap() {
                id::ADD_ENTITY => Sent::Add(r.varint().unwrap() as u32),
                id::BUNDLE_DELIMITER => Sent::End,
                id::REMOVE_ENTITIES => {
                    let n = r.varint().unwrap();
                    Sent::Remove((0..n).map(|_| r.varint().unwrap() as u32).collect())
                }
                id::CUSTOM_PAYLOAD if r.string(64).unwrap() == CHANNEL => {
                    Sent::Said(String::from_utf8(r.rest().to_vec()).unwrap())
                }
                other => Sent::Other(other),
            }
        })
        .collect()
}

/// Two loaded guests in chunk (0, 0) on a vanilla peer's connection each, and two items near
/// them that never come within pickup reach: `run` gets the game, each guest's packet reader and
/// the items' ids.
pub(crate) fn with_two_items(
    run: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        &dyn Fn() -> Vec<Heard>,
        [&dyn Fn() -> Vec<Sent>; 2],
        [u32; 2],
    ),
) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let heard = || std::mem::take(&mut *log.lock().unwrap());
    let mods = vec![ModEntry::new(
        "tracktest",
        Box::new(Probe(Arc::clone(&log))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let transport = Transport::capture(
                Side::Server,
                crate::raw_payloads::connection::record_overhead(&[]),
            )
            .unwrap();
            transport.accept(&[]).unwrap();
            transport.finish_configuration().unwrap();
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let mut readers = Vec::new();
            for conn in [1, 2] {
                let mut g = guest(conn);
                g.loaded = true;
                g.load_timeout = 0;
                g.position = [8.5, 64.0, 2.5 + f64::from(conn)];
                let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
                g.sender = sender;
                readers.push(out);
                game.server.lan.as_mut().unwrap().hub.guests.push(g);
            }
            let mut items = [0; 2];
            for (i, x) in [12.5, 4.5].into_iter().enumerate() {
                game.spawn_item(
                    ItemStack::new("minecraft:diamond", 1),
                    [x, 64.0, 8.5],
                    [0.0; 3],
                    32767,
                );
                items[i] = game.dropped.last().unwrap().id;
            }
            let (a, b) = (&readers[0], &readers[1]);
            run(
                game,
                &heard,
                [&move || read(a()), &move || read(b())],
                items,
            )
        })
    });
}

/// Run server ticks until a mod hears something, at most 20: a new guest is sent its chunks,
/// and pairs the entities in them, over its first ticks.
pub(crate) fn tick_until_heard(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    heard: &dyn Fn() -> Vec<Heard>,
) -> Vec<Heard> {
    for _ in 0..20 {
        // The tracker pairs only what the client was sent the chunk of (`isChunkTracked`); this
        // fixture runs no chunk sending, so chunk (0, 0) stands as sent.
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| {
                g.sent_chunks.insert((0, 0));
            });
        game.tick_lan(false);
        let told = heard();
        if !told.is_empty() {
            return told;
        }
    }
    let host = game.server.lan.as_ref().unwrap();
    let guests: Vec<_> = host
        .hub
        .guests
        .iter()
        .map(|g| (g.conn, g.sent_chunks.len(), g.tracker.len(), g.position))
        .collect();
    panic!("nothing heard in 20 ticks: (conn, chunks sent, tracked, position) {guests:?}");
}

/// The index of the first packet matching `wanted`.
pub(crate) fn at(sent: &[Sent], wanted: &Sent) -> usize {
    sent.iter()
        .position(|s| s == wanted)
        .unwrap_or_else(|| panic!("{wanted:?} not sent: {sent:?}"))
}

/// The stops a removal packet carries: it is followed at once by the mod's word for each,
/// in the order the mods heard them.
pub(crate) fn words_after_removal(sent: &[Sent], id: u32) -> (Vec<u32>, Vec<&Sent>) {
    let removed = sent
        .iter()
        .position(|s| matches!(s, Sent::Remove(ids) if ids.contains(&id)))
        .unwrap_or_else(|| panic!("no removal of {id}: {sent:?}"));
    let Sent::Remove(ids) = &sent[removed] else {
        unreachable!()
    };
    let words = sent[removed + 1..removed + 1 + ids.len()].iter().collect();
    (ids.clone(), words)
}

pub(crate) fn sorted(mut v: Vec<Heard>) -> Vec<Heard> {
    v.sort_by_key(|h| format!("{h:?}"));
    v
}

#[test]
#[ignore = "daily"]
fn starts_and_stops_are_heard_at_their_packets() {
    with_two_items(|game, heard, [first, second], [far, near]| {
        // `guest` gives connection `conn` the entity id `conn + 19`: the players track each other.
        let (one, two) = (20, 21);
        // Each guest starts tracking each item and the other player, all still in the level.
        assert_eq!(
            sorted(tick_until_heard(game, heard)),
            [
                Heard::Start(far, true),
                Heard::Start(far, true),
                Heard::Start(near, true),
                Heard::Start(near, true),
                Heard::Start(one, true),
                Heard::Start(two, true),
            ]
        );
        for (sent, other) in [(first(), two), (second(), one)] {
            for entity in [far, near, other] {
                // The mod's word follows the packets that add the entity, before the next
                // entity's.
                let added = at(&sent, &Sent::Add(entity));
                let said = at(&sent, &Sent::Said(format!("start {entity}")));
                assert!(
                    said > added
                        && sent[added + 1..said]
                            .iter()
                            .all(|s| matches!(s, Sent::Other(_) | Sent::End)),
                    "{sent:?}"
                );
            }
        }

        // The first guest leaves everyone's range, and the near item leaves the level.
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.position = [4000.5, 64.0, 8.5]);
        let index = game.dropped.iter().position(|d| d.id == near).unwrap();
        let _ = game.dropped.remove(index);
        assert_eq!(
            sorted(tick_until_heard(game, heard)),
            [
                Heard::Stop(far),
                Heard::Stop(near),
                Heard::Stop(near),
                Heard::Stop(one),
                Heard::Stop(two),
            ]
        );
        // Each player is sent one removal for what it stopped tracking, then the mod's words.
        for (sent, gone) in [(first(), vec![far, near, two]), (second(), vec![near, one])] {
            let (mut ids, words) = words_after_removal(&sent, near);
            ids.sort();
            assert_eq!(ids, gone);
            let mut words: Vec<_> = words.into_iter().collect();
            words.sort_by_key(|w| format!("{w:?}"));
            let mut expected: Vec<_> = gone
                .iter()
                .map(|id| Sent::Said(format!("stop {id}")))
                .collect();
            expected.sort_by_key(|w| format!("{w:?}"));
            assert_eq!(words, expected.iter().collect::<Vec<_>>(), "{sent:?}");
        }
        // The second guest still tracks the far item: nothing about it.
        assert!(!second().contains(&Sent::Said(format!("stop {far}"))));
    });
}

#[test]
#[ignore = "daily"]
fn a_leaving_player_hears_its_stops_after_logging_out() {
    with_two_items(|game, heard, [first, _], [far, near]| {
        let _ = (tick_until_heard(game, heard), first());
        let mut host = game.server.lan.take().unwrap();
        // The hub takes a leaving guest out of its list, and what it tracked stops
        // (`Hub::remove_guest`, `PlayerList.remove`), before it reports the leave.
        let mut gone = host.hub.guests.remove(0);
        gone.tracker.clear();
        game.server.lan = game.lan_process_events(
            host,
            [HostEvent::Left(Box::new(gone))],
            &[],
            [8.5, 64.0, 8.5],
        );
        let told = heard();
        assert_eq!(told[0], Heard::LoggedOut, "{told:?}");
        let mut stops: Vec<_> = told[1..].iter().collect();
        stops.sort_by_key(|h| format!("{h:?}"));
        // The other player (entity 21) is an entity it tracked too.
        assert_eq!(
            stops,
            [&Heard::Stop(far), &Heard::Stop(near), &Heard::Stop(21)]
        );
    });
}
