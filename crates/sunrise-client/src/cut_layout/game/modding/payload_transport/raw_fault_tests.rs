#![cfg(any())]

//! Main-thread fault isolation on one raw channel (#3209), through a published server and a
//! vanilla-shaped TCP peer that never answers the Sunrise payload query: the first declaring
//! mod stages a write and fails, the second commits its own; the first's write is rolled back
//! before the second runs, each runs once per message in load order, and the connection stays.
#[cfg(test)]
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, Registrar,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sunrise_protocol::{
    connection::Connection,
    packets::{Empty, configuration as cfg, play},
};

const NAME: &str = "cut_layout::game::payload_transport::raw_fault_tests::a_failing_raw_handler_rolls_back_alone_and_the_next_one_commits";
const CHANNEL: &str = "faultprobe:message";
const PLAYER: &str = "RawFault";
const STAGED: f32 = 3.0;
const COMMITTED: f32 = 7.0;

/// (mod, round, health the handler saw on entry)
type Log = Arc<Mutex<Vec<(&'static str, u8, f32)>>>;

struct Step {
    name: &'static str,
    fails: bool,
    log: Log,
}
impl IPayloadHandler for Step {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        let round = bytes.0[0];
        let player = context
            .player()
            .map_err(|e| mod_api::PayloadHandleError::Failed(format!("{e:?}")))?;
        let host = context
            .server_host()
            .map_err(|e| mod_api::PayloadHandleError::Failed(format!("{e:?}")))?;
        let living = host
            .living(player.entity())
            .ok_or_else(|| mod_api::PayloadHandleError::Failed("no living player".into()))?;
        let seen = living
            .health(host)
            .ok_or_else(|| mod_api::PayloadHandleError::Failed("no health".into()))?;
        self.log.lock().unwrap().push((self.name, round, seen));
        if self.fails {
            assert!(living.set_health(host, STAGED));
            assert_eq!(
                living.health(host),
                Some(STAGED),
                "the write is staged in place"
            );
            // A failed delivery discards its explicit disconnect with its writes.
            context.disconnect(mod_api::Text::literal("discard this failed handler's kick"));
            return Err(mod_api::PayloadHandleError::Failed(
                "refused after staging a write".into(),
            ));
        }
        assert!(living.set_health(host, COMMITTED));
        context
            .reply(&PayloadId(CHANNEL.into()), bytes)
            .map(drop)
            .map_err(|e| mod_api::PayloadHandleError::Failed(format!("reply: {e:?}")))
    }
}
struct Declares(Arc<Step>);
#[mod_api::__subscriptions]
impl Hooks for Declares {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_raw(PayloadId(CHANNEL.into()), Some(self.0.clone()));
    }
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn a_failing_raw_handler_rolls_back_alone_and_the_next_one_commits() {
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_RAW_FAULT_CHILD",
        &["vanilla_peer"],
    ) {
        return;
    }
    let log = Log::default();
    let step = |name, fails| {
        Box::new(Declares(Arc::new(Step {
            name,
            fails,
            log: log.clone(),
        })))
    };
    let mods = vec![
        ModEntry::new("faultfirst", step("faultfirst", true)),
        ModEntry::new("faultsecond", step("faultsecond", false)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.client.settings.render_distance = 2;
            game.client.settings.simulation_distance = 2;
            let options = sunrise_sim::world_options::WorldOptions {
                world_type: sunrise_sim::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "raw_fault",
                Some("Raw fault"),
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
            let port = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    server.publish(sunrise_gameplay::game::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    })
                })
                .expect("integrated server answered publication request")
                .unwrap();
            let mut peer = connect_vanilla_peer(port);
            // The second message's reply proves the first failure left the connection open.
            for round in [1u8, 2] {
                peer.send(
                    &play::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: vec![round],
                    })
                    .encode(),
                )
                .unwrap();
                await_reply(&mut peer, round);
            }
            let full = log.lock().unwrap()[0].2;
            assert_eq!(
                *log.lock().unwrap(),
                [
                    ("faultfirst", 1, full),
                    // The first mod's staged write was rolled back before the second ran.
                    ("faultsecond", 1, full),
                    // The second mod's write was committed.
                    ("faultfirst", 2, COMMITTED),
                    ("faultsecond", 2, COMMITTED),
                ]
            );
            assert_ne!(full, STAGED);
            assert_ne!(full, COMMITTED);
            drop(peer);
            game.leave_server(None).unwrap();
        });
    });
}

fn await_reply(c: &mut Connection, round: u8) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "raw reply deadline: round {round}"
        );
        let (id, body) = c.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                assert_eq!(p.data, [round]);
                return;
            }
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
}

/// Logs in as a vanilla client would: no Sunrise payload offer, known packs answered at once.
fn connect_vanilla_peer(port: u16) -> Connection {
    use sunrise_protocol::packets::{handshake, login};
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
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
            name: PLAYER.into(),
            uuid: sunrise_protocol::crypto::offline_uuid(PLAYER),
        })
        .encode(),
    )
    .unwrap();
    loop {
        assert!(Instant::now() < deadline, "login deadline");
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    loop {
        assert!(Instant::now() < deadline, "configuration deadline");
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::SelectKnownPacks(p) => c
                .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                .unwrap(),
            cfg::Clientbound::FinishConfiguration(_) => break,
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    loop {
        assert!(Instant::now() < deadline, "play teleport deadline");
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

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
