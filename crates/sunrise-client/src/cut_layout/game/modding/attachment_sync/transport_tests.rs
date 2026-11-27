//! Attachment sync on the negotiated route (#813 step 4a), over the real integrated memory
//! connection and LAN TCP: who is sent values, in which order, what a rollback and a stale
//! epoch send, and what an oversized message does to the connection.
use mod_api::AttachmentCodec;
#[cfg(any())]
use mod_api::{AttachmentAccess, AttachmentHolder, Attachments, Host};
#[cfg(any())]
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use sunrise_core::nbt::Tag;
#[cfg(any())]
use sunrise_mod_runtime::attachments::Holder;
#[cfg(any())]
use sunrise_protocol::{
    attachment_sync,
    buf::{Codec, Ident, Reader},
    connection::Connection,
    packets::{Empty, configuration as cfg, handshake, login, play},
    payload_negotiation::{self as negotiation, Control},
};

#[cfg(any())]
const NAME: &str = "cut_layout::game::server_side::attachment_sync::transport_tests::attachment_sync_follows_the_negotiated_route";

pub(super) const VALUE_CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
    decode: |c| match c.get(&"n".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(mod_api::AttachmentError::Decode),
    },
};

#[cfg(any())]
const BLOB_CODEC: AttachmentCodec<Vec<u8>> = AttachmentCodec {
    encode: |bytes| Ok([("b".into(), Tag::ByteArray(bytes.clone()))].into()),
    decode: |c| match c.get(&"b".into()) {
        Some(Tag::ByteArray(bytes)) => Ok(bytes.clone()),
        _ => Err(mod_api::AttachmentError::Decode),
    },
};

#[cfg(any())]
pub(super) fn value_key() -> mod_api::AttachmentKey<i32> {
    mod_api::testing::attachment_key("sync_route", "value")
}

// This is installed only by the fresh-process route fixtures. The disk decoder creates the
// throwing owner, not the setup write. The encoder unwinds with another throwing destructor.
#[cfg(any())]
static PANICKING_DROPS: AtomicUsize = AtomicUsize::new(0);
#[cfg(any())]
#[derive(Clone)]
struct PanickingValue(bool);
#[cfg(any())]
impl Drop for PanickingValue {
    fn drop(&mut self) {
        if self.0 {
            PANICKING_DROPS.fetch_add(1, Ordering::Relaxed);
            panic!("disk-decoded attachment destructor");
        }
    }
}

/// A mod with no hooks: with a mod loaded, the server sends joining players snapshots.
#[cfg(any())]
pub(super) struct Route;
#[cfg(any())]
#[mod_api::__subscriptions]
impl mod_api::Hooks for Route {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .register_attachment_type(
                mod_api::AttachmentDescriptor::new("sync_route:value", || 0i32)
                    .serialize(VALUE_CODEC)
                    .sync(VALUE_CODEC),
            )
            .unwrap();
        registrar
            .register_attachment_type(
                mod_api::AttachmentDescriptor::new("sync_route:blob", Vec::<u8>::new)
                    .serialize(BLOB_CODEC)
                    .sync(BLOB_CODEC),
            )
            .unwrap();
        registrar
            .register_attachment_type(
                mod_api::AttachmentDescriptor::new("sync_route:bad_nbt", || ()).sync(
                    AttachmentCodec {
                        encode: |_| {
                            Ok([(
                                "too_long".into(),
                                Tag::String("x".repeat(65_536).as_str().into()),
                            )]
                            .into())
                        },
                        decode: |_| Ok(()),
                    },
                ),
            )
            .unwrap();
        registrar
            .register_attachment_type(
                mod_api::AttachmentDescriptor::new("sync_route:panic", || PanickingValue(false))
                    .serialize(AttachmentCodec {
                        encode: |_| Ok(Default::default()),
                        decode: |_| Ok(PanickingValue(true)),
                    })
                    .sync(AttachmentCodec {
                        encode: |_| std::panic::panic_any(PanickingValue(true)),
                        decode: |_| unreachable!("the panicking encoder produced no payload"),
                    }),
            )
            .unwrap();
    }
}

/// The server sets the level's value, committed or rolled back.
#[cfg(any())]
pub(super) fn set_level(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    value: i32,
    commit: bool,
) {
    let key = value_key();
    game.client
        .integrated
        .as_ref()
        .unwrap()
        .call(move |server| {
            server.storage_scope(|server, _| {
                key.set(server, Holder::Level, value).unwrap();
                ((), commit)
            });
        })
        .unwrap();
}

/// The window's synced copy of the level's value.
#[cfg(any())]
fn window_value(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    key: &mod_api::AttachmentKey<i32>,
) -> Option<i32> {
    let host: &dyn Host = game;
    let holder = AttachmentHolder::Level(host.level(host.dimension())?);
    let batch = mod_api::AttachmentQueries::capture(host, key).unwrap();
    batch
        .query(key)
        .unwrap()
        .get(game.attachment_frame_borrow(), holder, |value| {
            value.cloned()
        })
        .unwrap()
}

/// Let the server's hub tick `n` times, so later edits land in a later pass.
#[cfg(any())]
pub(super) fn wait_server_ticks(game: &sunrise_client_gameplay::game::ClientGame<'_>, n: u64) {
    let ticks = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
        game.client
            .integrated
            .as_ref()
            .unwrap()
            .call(|server| server.server.lan.as_ref().unwrap().hub.ticks)
            .unwrap()
    };
    let start = ticks(game);
    let deadline = Instant::now() + Duration::from_secs(30);
    while ticks(game) < start + n {
        assert!(Instant::now() < deadline, "the server stopped ticking");
        std::thread::yield_now();
    }
}

#[cfg(any())]
fn wait_window(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    key: &mod_api::AttachmentKey<i32>,
    value: Option<i32>,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while window_value(game, key) != value {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "window never saw {value:?}");
        std::thread::yield_now();
    }
}

/// A TCP peer that offers the engine's declarations (`sunrise`) or offers nothing, as a
/// vanilla client does, logged in and in play.
#[cfg(any())]
pub(super) fn peer(port: u16, name: &str, sunrise: bool) -> Connection {
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: sunrise_protocol::PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: sunrise_protocol::crypto::offline_uuid(name),
        })
        .encode(),
    )
    .unwrap();
    loop {
        assert!(Instant::now() < deadline, "login deadline: {name}");
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    let local =
        sunrise_sim::raw_payloads::Local::capture(sunrise_sim::raw_payloads::Side::Client).unwrap();
    if sunrise {
        // A server queries a client only once its brand says Sunrise.
        let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
        brand.string("sunrise");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: brand.data,
            })
            .encode(),
        )
        .unwrap();
    }
    loop {
        assert!(Instant::now() < deadline, "configuration deadline: {name}");
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::CustomPayload(p) if p.channel == negotiation::CHANNEL => {
                if sunrise && Control::decode(&p.data).unwrap() == Control::Query {
                    let offer = Control::Offer(local.manifest.clone()).encode().unwrap();
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: negotiation::CHANNEL.into(),
                            data: offer,
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            cfg::Clientbound::SelectKnownPacks(p) => {
                c.send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                    .unwrap();
            }
            // The server pings a Sunrise client whose Offer is not in by its pack reply.
            cfg::Clientbound::Ping(p) => c.send(&cfg::Serverbound::Pong(p).encode()).unwrap(),
            cfg::Clientbound::FinishConfiguration(_) => break,
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    loop {
        assert!(Instant::now() < deadline, "play deadline: {name}");
        let (id, body) = c.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::PlayerPosition(p) => {
                c.send(
                    &play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: p.yaw,
                        pitch: p.pitch,
                    })
                    .encode(),
                )
                .unwrap();
                c.send(&play::Serverbound::PlayerLoaded(Empty).encode())
                    .unwrap();
                return c;
            }
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
}

/// The values of `holder` a peer is sent until it is sent `last`, in order. A disconnect
/// fails.
#[cfg(any())]
pub(super) fn values_until(c: &mut Connection, holder: Holder, last: i32) -> Vec<i32> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while seen.last() != Some(&last) {
        assert!(Instant::now() < deadline, "never sent {last}: {seen:?}");
        let (id, body) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => panic!("peer receive failed: {error}"),
        };
        if id == sunrise_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
            let mut reader = Reader::new(&body);
            if Ident::read(&mut reader).unwrap() == attachment_sync::CHANNEL {
                let message = attachment_sync::decode(reader.rest()).unwrap();
                if message.holder == holder {
                    seen.extend(message.changes.iter().filter_map(|change| {
                        match change.value.as_ref()?.get(&"n".into())? {
                            Tag::Int(n) if change.name.to_string().ok()? == "value" => Some(*n),
                            _ => None,
                        }
                    }));
                }
                continue;
            }
        }
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    seen
}

/// How many attachment sync payloads a peer is sent in `window`; a disconnect fails.
#[cfg(any())]
pub(super) fn sync_packets_within(c: &mut Connection, window: Duration) -> usize {
    c.reader
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + window;
    let mut count = 0;
    while Instant::now() < deadline {
        let (id, body) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => panic!("peer receive failed: {error}"),
        };
        if id == sunrise_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
            let mut reader = Reader::new(&body);
            if Ident::read(&mut reader).unwrap() == attachment_sync::CHANNEL {
                count += 1;
                continue;
            }
        }
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    count
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn attachment_sync_follows_the_negotiated_route() {
    std::env::var_os("SUNRISE_TEST_RESOURCES")
        .expect("missing SUNRISE_TEST_RESOURCES: a prepared 26.3 resource root");
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_ATTACHMENT_SYNC_ROUTE_CHILD",
        &["route"],
    ) {
        return;
    }
    let mods = vec![mod_api::ModEntry::new("sync_route", Box::new(Route))].into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let value_key = value_key();
            game.client.settings.render_distance = 2;
            game.client.settings.simulation_distance = 2;
            let options = sunrise_sim::world_options::WorldOptions {
                world_type: sunrise_sim::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "sync_route",
                Some("Sync route"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let set_pair = |game: &sunrise_client_gameplay::game::ClientGame<'_>, n: i32| {
                let bad = mod_api::testing::attachment_key("sync_route", "bad_nbt");
                let good = value_key.clone();
                game.client
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(move |server| {
                        server.storage_scope(|server, _| {
                            bad.set(server, Holder::Level, ()).unwrap();
                            good.set(server, Holder::Level, n).unwrap();
                            ((), true)
                        });
                    })
                    .unwrap();
            };
            // Integrated memory connection: a commit reaches the window, a rollback does not.
            set_level(game, 7, true);
            wait_window(game, &value_key, Some(7));
            set_level(game, 9, false);
            set_pair(game, 10);
            wait_window(game, &value_key, Some(10));

            // Leave a decoded-on-send, saved attachment on the authoritative level before the
            // TCP peer pairs. A temporary owner and the encoder's panic payload both unwind.
            let panicking = mod_api::testing::attachment_key("sync_route", "panic");
            game.client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |server| {
                    panicking
                        .set(server, Holder::Level, PanickingValue(false))
                        .unwrap();
                    let map = server
                        .attachment_map_quiet(Holder::Level, AttachmentAccess::Existing)
                        .unwrap();
                    map.replace_contents(Attachments::from_nbt(map.try_to_nbt().unwrap()));
                })
                .unwrap();

            let port = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    server.publish(sunrise_gameplay::game::server_side::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    })
                })
                .unwrap()
                .unwrap();
            let mut sunrise = peer(port, "SyncSunrise", true);
            let mut vanilla = peer(port, "SyncVanilla", false);
            // A joining Sunrise peer is sent the level's whole value, then each commit in
            // order; a refused projection on that same holder does not suppress the join
            // snapshot or a later commit. A rolled-back edit sends nothing.
            assert_eq!(values_until(&mut sunrise, Holder::Level, 10), [10]);
            assert_eq!(
                PANICKING_DROPS.load(Ordering::Relaxed),
                2,
                "the failed encoder's panic payload and temporary disk-decoded owner were fenced independently"
            );
            set_level(game, 11, false);
            let ordered_key = value_key.clone();
            game.client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |server| {
                    server.storage_scope(|server, _| {
                        ordered_key.set(server, Holder::Level, 120).unwrap();
                        server.storage_scope(|server, _| {
                            ordered_key.set(server, Holder::Level, 999).unwrap();
                            ((), false)
                        });
                        ordered_key.set(server, Holder::Level, 12).unwrap();
                        ((), true)
                    });
                })
                .unwrap();
            assert_eq!(values_until(&mut sunrise, Holder::Level, 12), [120, 12]);
            let authority = {
                let key = value_key.clone();
                game.client
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(move |server| key.get(server, Holder::Level, Clone::clone))
                    .unwrap()
            };
            assert_eq!(authority, Ok(12));
            // A vanilla peer is sent nothing, before or after.
            assert_eq!(sync_packets_within(&mut vanilla, Duration::from_secs(2)), 0);

            let discard = |game: &sunrise_client_gameplay::game::ClientGame<'_>, commit| {
                let key = value_key.clone();
                game.client
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(move |server| {
                        server.storage_scope(|server, _| {
                            assert_eq!(key.remove(server, Holder::Level), Ok(Some(12)));
                            ((), commit)
                        });
                    })
                    .unwrap();
            };
            wait_window(game, &value_key, Some(12));
            discard(game, false);
            wait_server_ticks(game, 2);
            game.poll_remote().unwrap();
            assert_eq!(window_value(game, &value_key), Some(12));
            assert_eq!(sync_packets_within(&mut sunrise, Duration::from_secs(1)), 0);
            discard(game, true);
            wait_window(game, &value_key, None);

            // A replica may have a local default even though the server lost that key.
            // The server's map still holds the other attachments from this route.
            assert_eq!(value_key.get(game, Holder::Level, |value| *value), Ok(0));
            assert_eq!(window_value(game, &value_key), Some(0));
            let absent = value_key.clone();
            game.client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |server| {
                    server.storage_scope(|server, _| {
                        assert_eq!(absent.remove(server, Holder::Level), Ok(None));
                        ((), true)
                    });
                })
                .unwrap();
            wait_window(game, &value_key, None);

            let holder = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    use sunrise_gameplay_common::game::creatures::{SpawnReason, SpawnSite};
                    let pos = server
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|guest| guest.name == "SyncSunrise")
                        .unwrap()
                        .position;
                    let id = server
                        .spawn_mob(
                            "minecraft:cow",
                            pos,
                            true,
                            SpawnSite::Finalizing(SpawnReason::Command),
                        )
                        .unwrap();
                    Holder::Entity(id)
                })
                .unwrap();
            wait_server_ticks(game, 2);
            let good = value_key.clone();
            let bad = mod_api::testing::attachment_key("sync_route", "bad_nbt");
            game.client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |server| {
                    server.storage_scope(|server, _| {
                        bad.set(server, holder, ()).unwrap();
                        good.set(server, holder, 23).unwrap();
                        ((), true)
                    });
                })
                .unwrap();
            assert_eq!(values_until(&mut sunrise, holder, 23), [23]);

            // Past the network frame limits: refused by name, the peer stays joined and is
            // sent the next value. The memory connection has no frame limit.
            let blob_key = mod_api::testing::attachment_key("sync_route", "blob");
            let refused = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |server| {
                    let blob: Vec<u8> = (0u32..9 << 20)
                        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
                        .collect();
                    let message = attachment_sync::Message {
                        holder: Holder::Level,
                        replace: false,
                        changes: vec![sunrise_mod_runtime::attachments::AttachmentSync {
                            namespace: "sync_route".into(),
                            name: "blob".into(),
                            value: Some([("b".into(), Tag::ByteArray(blob.clone()))].into()),
                        }],
                    };
                    let body = attachment_sync::encode(&message).map_err(|e| e.to_string());
                    let peer = server
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.name == "SyncSunrise")
                        .unwrap()
                        .sender
                        .clone();
                    let refused = server.queue_attachment_sync(&peer, &body);
                    // The same value through the level's own pass, as a mod's edit sends it.
                    blob_key.set(server, Holder::Level, blob).unwrap();
                    refused
                })
                .unwrap();
            assert!(
                matches!(
                    refused,
                    Err(mod_api::PayloadSendError::FrameTooLarge { .. })
                ),
                "{refused:?}"
            );
            wait_server_ticks(game, 2);
            set_level(game, 13, true);
            assert_eq!(values_until(&mut sunrise, Holder::Level, 13), [13]);
            wait_window(game, &value_key, Some(13));

            // A message prepared against an epoch that ended before the root commit publishes
            // nothing, on the TCP and the memory connection alike.
            let stale = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    let senders: Vec<_> = server
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .filter(|g| g.name != "SyncVanilla")
                        .map(|g| g.sender.clone())
                        .collect();
                    let message = attachment_sync::Message {
                        holder: Holder::Level,
                        replace: false,
                        changes: vec![sunrise_mod_runtime::attachments::AttachmentSync {
                            namespace: "sync_route".into(),
                            name: "value".into(),
                            value: Some([("n".into(), Tag::Int(99))].into()),
                        }],
                    };
                    server.storage_scope(|server, _| {
                        server.send_attachment_sync(&senders, &message);
                        for sender in &senders {
                            sender.payload_transport().begin_configuration();
                        }
                        ((), true)
                    });
                    senders.len()
                })
                .unwrap();
            assert_eq!(stale, 2, "the window's and the Sunrise peer's connections");
            assert_eq!(sync_packets_within(&mut sunrise, Duration::from_secs(2)), 0);
            for _ in 0..20 {
                game.poll_remote().unwrap();
            }
            assert_eq!(window_value(game, &value_key), Some(13));
            drop((sunrise, vanilla));
            game.leave_server(None).unwrap();
        });
    });
}

#[cfg(any())]
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
