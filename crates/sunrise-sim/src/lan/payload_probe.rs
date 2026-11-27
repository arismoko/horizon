//! A hosted guest's raw-payload delivery, for tests that push one through the inbox.
use crate::lan::PlayerSender;
use crate::raw_payloads::{Classified, Delivery, Endpoint, Incoming};

pub(crate) fn delivery(conn: u32, sender: PlayerSender, bytes: Vec<u8>) -> Box<Delivery> {
    let classified = Incoming::classify(
        sender.payload_transport(),
        sunrise_protocol::payload_negotiation::Phase::Play,
        sunrise_protocol::packets::configuration::CustomPayload {
            channel: crate::raw_payloads::test_support::CHANNEL.into(),
            data: bytes,
        },
    )
    .unwrap();
    let Classified::Application(incoming) = classified else {
        panic!("fixture registration must classify as application");
    };
    Box::new(Delivery {
        conn: Some(conn),
        endpoint: Endpoint::server(sender),
        incoming,
    })
}
