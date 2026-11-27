//! A real LAN join: the virtual host a player reports is its accepted handshake's host,
//! after `player_handshake` listeners rewrite it.
#![cfg(any())]
#[cfg(test)]
use mod_api::ModEntry;
use mod_api::{Flow, Hooks, PlayerHandshakeEvent, ServerHost, VirtualHost};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

const NAME: &str = "cut_layout::game::payload_transport::virtual_host_tests::virtual_host_is_the_rewritten_handshake_host";

struct Rewrite(Arc<Mutex<Vec<Option<VirtualHost>>>>);
#[mod_api::__subscriptions]
impl Hooks for Rewrite {
    fn player_handshake(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerHandshakeEvent<'_>,
    ) -> Flow {
        if event.requested_host.as_str() == "join.example.test" {
            *event.requested_host = "rewritten.example.test\0suffix".into();
        }
        Flow::Continue
    }
    fn player_logged_in(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerLoggedInEvent<'_>,
    ) {
        let player = host
            .player(event.player)
            .expect("a logged-in player resolves");
        self.0
            .lock()
            .unwrap()
            .push(player.virtual_host(host).cloned());
    }
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn virtual_host_is_the_rewritten_handshake_host() {
    super::tests::require_resources();
    sunrise_gameplay_common::game::in_own_process(NAME, || {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("rewrite", Box::new(Rewrite(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                game.client.settings.render_distance = 2;
                game.client.settings.simulation_distance = 2;
                let options = sunrise_sim::world_options::WorldOptions {
                    world_type: sunrise_sim::save::WorldPreset::Flat,
                    ..Default::default()
                };
                game.open_integrated(
                    "virtual_host",
                    Some("Virtual host"),
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
                let c = join(port, "join.example.test", "HostPeer");
                let deadline = Instant::now() + Duration::from_secs(30);
                while !seen.lock().unwrap().iter().any(Option::is_some) {
                    assert!(Instant::now() < deadline, "player_logged_in never ran");
                    std::thread::yield_now();
                }
                // The window's own player joined first, through its memory connection's handshake.
                assert_eq!(
                    *seen.lock().unwrap(),
                    [
                        Some(VirtualHost {
                            host: "local".into(),
                            port: 0,
                        }),
                        Some(VirtualHost {
                            host: "rewritten.example.test".into(),
                            port,
                        }),
                    ]
                );
                drop(c);
                game.leave_server(None).unwrap();
            });
        });
    });
}

/// A vanilla-shaped client joining through `host` until the play login arrives.
fn join(port: u16, host: &str, name: &str) -> sunrise_protocol::connection::Connection {
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, handshake, login, play},
        payload_negotiation::{CHANNEL, Control},
    };
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: sunrise_protocol::PROTOCOL_VERSION,
            host: host.into(),
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
    loop {
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::SelectKnownPacks(p) => c
                .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                .unwrap(),
            cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                if let Control::Query = Control::decode(&p.data).unwrap() {
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Offer(local.manifest.clone()).encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            cfg::Clientbound::FinishConfiguration(_) => break,
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    loop {
        let (id, body) = c.recv().unwrap();
        if let play::Clientbound::Login(_) = play::Clientbound::decode(id, &body).unwrap() {
            return c;
        }
    }
}

#[cfg(test)]
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
