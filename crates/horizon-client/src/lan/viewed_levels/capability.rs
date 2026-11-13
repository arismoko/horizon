//! The viewed-levels capability, server side. In configuration the server asks a client whose
//! brand is Horizon's how many levels besides its own it can view, and keeps the answer for play
//! (`PlayerSender::max_viewed_levels`). A vanilla client is never asked. A client that does not
//! answer before it acknowledges `FinishConfiguration`, or answers another revision, views none.
//! An answer the server did not ask for, or a second one, closes the connection.
use horizon_protocol::{
    packets::configuration as cfg,
    viewed_levels::{self, ViewedLevelsError},
};

/// One configuration phase's exchange.
#[derive(Default)]
pub struct Capability {
    asked: bool,
    answered: bool,
    max_levels: u8,
}

impl Capability {
    /// The query for a Horizon-branded client.
    pub fn query(&mut self) -> cfg::CustomPayload {
        self.asked = true;
        cfg::CustomPayload {
            channel: viewed_levels::CHANNEL.into(),
            data: viewed_levels::query(),
        }
    }

    /// The client's answer, `data` after the channel.
    pub fn receive(&mut self, data: &[u8]) -> Result<(), ViewedLevelsError> {
        if !self.asked {
            return Err(ViewedLevelsError::UnaskedAnswer);
        }
        if self.answered {
            return Err(ViewedLevelsError::SecondAnswer);
        }
        self.answered = true;
        self.max_levels = viewed_levels::read_answer(data)?;
        Ok(())
    }

    /// How many levels besides its own the client can view: 0 until it answers.
    pub fn max_levels(&self) -> u8 {
        self.max_levels
    }
}

#[cfg(test)]
mod tests {
    use crate::lan::{Lan, NetConfig, NetEvent, data::SyncData};
    use horizon_protocol::{
        buf::Writer,
        connection::Connection,
        crypto::offline_uuid,
        packets::{Empty, configuration as cfg, handshake, login},
        payload_negotiation::{self as negotiation, Control, Manifest},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    /// What a client saw and what the server kept.
    #[derive(Debug, PartialEq)]
    struct Joined {
        /// The `horizon:levels` query it was sent.
        asked: Option<Vec<u8>>,
        /// The server's `max_viewed_levels` for it once it plays, or the reason it was closed.
        kept: Result<u8, String>,
    }

    fn payload(channel: &str, data: Vec<u8>) -> Vec<u8> {
        cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: channel.into(),
            data,
        })
        .encode()
    }

    /// A client with `brand` joins a server: it sends `unasked` payloads on `horizon:levels` with
    /// its brand, and `answers` once it is asked.
    fn join(brand: &str, unasked: &[&[u8]], answers: &[&[u8]]) -> Joined {
        let config = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::lan(0, "Viewed levels".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        for packet in [
            handshake::Serverbound::Intention(handshake::Intention {
                protocol: horizon_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
            login::Serverbound::Hello(login::ServerHello {
                name: "Viewer".into(),
                uuid: offline_uuid("Viewer"),
            })
            .encode(),
        ] {
            c.send(&packet).unwrap();
        }
        loop {
            let (id, body) = c.recv().unwrap();
            match login::Clientbound::decode(id, &body).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => break,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
        c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
            .unwrap();
        let mut name = Writer::new();
        name.string(brand);
        c.send(&payload("minecraft:brand", name.data)).unwrap();
        if brand == "horizon" {
            // As a Horizon client's net thread: the raw payload protocol, with no mod payloads.
            let offer = Control::Offer(Manifest::builder().finish().unwrap());
            for control in [Control::Query, offer] {
                c.send(&payload(negotiation::CHANNEL, control.encode().unwrap()))
                    .unwrap();
            }
        }
        for data in unasked {
            c.send(&payload("horizon:levels", data.to_vec())).unwrap();
        }
        let reason =
            |d: cfg::Disconnect| horizon_protocol::text::plain(&d.reason, &Default::default());
        let mut asked = None;
        loop {
            let (id, body) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &body).unwrap() {
                cfg::Clientbound::CustomPayload(p) if p.channel == "horizon:levels" => {
                    asked = Some(p.data);
                    for data in answers {
                        c.send(&payload("horizon:levels", data.to_vec())).unwrap();
                    }
                }
                cfg::Clientbound::SelectKnownPacks(p) => {
                    let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                }
                cfg::Clientbound::Ping(p) => {
                    let _ = c.send(&cfg::Serverbound::Pong(p).encode());
                }
                cfg::Clientbound::Disconnect(d) => {
                    return Joined {
                        asked,
                        kept: Err(reason(d)),
                    };
                }
                cfg::Clientbound::FinishConfiguration(_) => {
                    // A server that refused the answers may have closed already.
                    let _ = c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode());
                    break;
                }
                _ => {}
            }
        }
        // The server sends FinishConfiguration before it reads the answers, so a refusal can
        // follow it: wait for either the refusal or the player handed to play.
        c.reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for receipt in lan.poll() {
                if let (NetEvent::Joined(j), _) = receipt.into_parts() {
                    return Joined {
                        asked,
                        kept: Ok(j.sender.max_viewed_levels),
                    };
                }
            }
            if let Ok((id, body)) = c.recv()
                && let Ok(cfg::Clientbound::Disconnect(d)) = cfg::Clientbound::decode(id, &body)
            {
                return Joined {
                    asked,
                    kept: Err(reason(d)),
                };
            }
        }
        panic!("the server neither refused the player nor handed it to play");
    }

    #[test]
    fn a_horizon_client_is_asked_and_its_answer_is_kept() {
        assert_eq!(
            join("horizon", &[], &[&[1, 1, 2]]),
            Joined {
                asked: Some(vec![0, 1]),
                kept: Ok(2),
            }
        );
    }

    #[test]
    fn a_vanilla_client_is_never_asked() {
        assert_eq!(
            join("vanilla", &[], &[]),
            Joined {
                asked: None,
                kept: Ok(0),
            }
        );
    }

    #[test]
    fn an_older_horizon_client_that_does_not_answer_views_none() {
        assert_eq!(
            join("horizon", &[], &[]),
            Joined {
                asked: Some(vec![0, 1]),
                kept: Ok(0),
            }
        );
    }

    #[test]
    fn an_answer_of_another_revision_views_none() {
        assert_eq!(
            join("horizon", &[], &[&[1, 2, 5]]),
            Joined {
                asked: Some(vec![0, 1]),
                kept: Ok(0),
            }
        );
    }

    #[test]
    fn a_second_answer_closes_the_connection() {
        assert_eq!(
            join("horizon", &[], &[&[1, 1, 2], &[1, 1, 2]]),
            Joined {
                asked: Some(vec![0, 1]),
                kept: Err("horizon:levels: a second answer".into()),
            }
        );
    }

    #[test]
    fn an_answer_nobody_asked_for_closes_the_connection() {
        assert_eq!(
            join("vanilla", &[&[1, 1, 2]], &[]),
            Joined {
                asked: None,
                kept: Err("horizon:levels: an answer the server did not ask for".into()),
            }
        );
    }

    struct Claims;
    impl mod_api::IPayloadHandler for Claims {
        fn handle(
            &self,
            _: mod_api::PayloadBytes,
            _: &mut dyn mod_api::IPayloadContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Claims {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            registrar
                .payload_handlers()
                .registrar("1")
                .configuration_to_server(
                    mod_api::PayloadId("horizon:levels".into()),
                    Arc::new(Claims),
                );
        }
    }

    #[test]
    fn a_mod_cannot_register_the_channel() {
        let mods = vec![mod_api::ModEntry::new("horizon", Box::new(Claims))];
        mod_platform::testing::with_mods(mods.into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let refused =
                crate::raw_payloads::Transport::capture(crate::raw_payloads::Side::Server)
                    .err()
                    .unwrap();
            assert_eq!(
                refused,
                "payload horizon:levels is reserved for an engine service or transport control"
            );
        });
    }
}
