//! Raw plugin channels against a vanilla-shaped peer (no Sunrise offer): sends,
//! fan-out to every declaring mod in load order, the vanilla size limits in both
//! directions, undeclared channels and the load-time collision refusal.
use crate::lan::{NetEvent, PlayerSender};
use crate::raw_payloads::vanilla_peer_support::{
    CHANNEL, Declares, SEND_ONLY, Typed, serverbound, with_vanilla_peer,
};
use crate::raw_payloads::*;
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, PayloadSendError,
    Registrar,
};
use std::sync::Arc;
use sunrise_protocol::packets::play;
use sunrise_protocol::payload_negotiation::Phase;

#[test]
fn a_raw_channel_reaches_every_declaring_mod_in_load_order() {
    mod_platform::__private::initialize().unwrap();
    with_vanilla_peer(|transport| {
        let (sender, _out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let (events, receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let Ok(Classified::Raw(deliveries)) =
            Incoming::classify(&transport, Phase::Play, serverbound(CHANNEL, 5))
        else {
            panic!("a declared raw channel is the mods'");
        };
        let endpoint = Endpoint::server(sender);
        for incoming in deliveries {
            assert!(incoming.raw);
            dispatch_server(
                incoming,
                endpoint.clone(),
                ServerQueue::new(events.clone(), endpoint.clone(), Some(1)),
            );
        }
        let main = ServerQueue::new(events.clone(), endpoint.clone(), Some(1));
        let mut owners = Vec::new();
        for _ in 0..2 {
            let turn = receiver.take_turn();
            assert_eq!(turn.len(), 1, "one main delivery per connection turn");
            for receipt in turn {
                match receipt.into_parts().0 {
                    NetEvent::RawPayload(delivery) => {
                        let Body::Payload(bytes) = &delivery.incoming.body else {
                            panic!("a message, not work");
                        };
                        assert_eq!(bytes.0, vec![7; 5]);
                        owners.push(delivery.incoming.receiver.owner());
                    }
                    _ => panic!("only raw deliveries were queued"),
                }
            }
            // Dropping the main delivery releases its receiving turn. The pooled
            // receiver's maintenance turn, not that drop, dispatches the next one.
            dispatch_pending_server(&endpoint, &main);
        }
        assert!(receiver.take_turn().is_empty());
        assert_eq!(owners, ["voicechat", "voicebridge"]);
        // Undeclared and send-only channels keep their unrelated-plugin behaviour.
        for channel in [SEND_ONLY, "other:channel"] {
            assert!(matches!(
                Incoming::classify(&transport, Phase::Play, serverbound(channel, 1)),
                Ok(Classified::Other(_))
            ));
        }
    });
}

#[test]
fn raw_sends_reach_a_vanilla_peer_within_the_clientbound_limit() {
    with_vanilla_peer(|transport| {
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let epoch = transport.epoch().unwrap();
        let id = PayloadId(SEND_ONLY.into());
        let body = b"\x00\x07Connect\x00\x05lobby".to_vec();
        endpoint
            .prepare(&epoch, &id, PayloadBytes(body.clone()))
            .unwrap()
            .publish()
            .unwrap();
        // Play clientbound custom_payload (0x18), the channel as a length-prefixed string,
        // then the body with no envelope: written out by hand, not by the encoder under test.
        let mut expected = vec![0x18, 15];
        expected.extend_from_slice(b"bungeecord:main");
        expected.extend_from_slice(&body);
        assert!(out().contains(&expected));
        // ClientboundCustomPayloadPacket's limit, as a literal so a changed constant fails here.
        assert!(
            endpoint
                .prepare(&epoch, &id, PayloadBytes(vec![0; 1_048_576]))
                .is_ok()
        );
        assert_eq!(
            endpoint
                .prepare(&epoch, &id, PayloadBytes(vec![0; 1_048_577]))
                .err(),
            Some(PayloadSendError::PayloadTooLarge {
                id: id.clone(),
                size: 1_048_577,
                limit: 1_048_576,
            })
        );
        let undeclared = PayloadId("other:channel".into());
        assert_eq!(
            endpoint
                .prepare(&epoch, &undeclared, PayloadBytes(vec![1]))
                .err(),
            Some(PayloadSendError::UnknownPayload(undeclared))
        );
    });
}

#[test]
fn a_raw_channel_keeps_the_vanilla_serverbound_limit() {
    with_vanilla_peer(|transport| {
        let decode = |len: usize| {
            let mut body = ::sunrise_core::vanilla_bytes::Writer::new();
            body.string(CHANNEL);
            body.bytes(&vec![0; len]);
            play::Serverbound::decode_with_payload_codecs(
                sunrise_protocol::ids::play::serverbound::CUSTOM_PAYLOAD,
                &body.data,
                &transport.local.codecs,
            )
        };
        // DiscardedPayload's serverbound limit, as a literal.
        assert!(decode(32_767).is_ok());
        assert!(decode(32_768).is_err());
    });
}

#[test]
fn a_raw_declaration_colliding_with_a_typed_payload_refuses_load() {
    let mods = vec![
        ModEntry::new("voicechat", Box::new(Typed)),
        ModEntry::new(
            "voicebridge",
            Box::new(Declares {
                receives: true,
                send_only: false,
            }),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        let refused =
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap_err();
        assert!(
            refused.iter().any(|line| line.contains("voicebridge")
                && line.contains(CHANNEL)
                && line.contains("voicechat")),
            "{refused:?}"
        );
    });
}

#[derive(Clone, Copy)]
enum Fault {
    Failed,
    Panic,
    Disconnect,
}
struct FaultHandler {
    fault: Option<Fault>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl IPayloadHandler for FaultHandler {
    fn handle(
        &self,
        _: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        use std::sync::atomic::Ordering;
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            Some(Fault::Failed) => {
                context.disconnect(mod_api::Text::literal("must be discarded on failure"));
                Err(mod_api::PayloadHandleError::Failed(
                    "deliberate failure".into(),
                ))
            }
            Some(Fault::Panic) => {
                context.disconnect(mod_api::Text::literal("must be discarded on panic"));
                panic!("deliberate raw handler panic");
            }
            Some(Fault::Disconnect) => {
                context.disconnect(mod_api::Text::literal("successful disconnect"));
                Ok(())
            }
            None => Ok(()),
        }
    }
}
struct FaultRegistration(Arc<FaultHandler>);
#[mod_api::__subscriptions]
impl Hooks for FaultRegistration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_raw(PayloadId(CHANNEL.into()), Some(self.0.clone()));
    }
}

#[test]
fn network_raw_failures_drop_only_the_failing_copy() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for fault in [Fault::Failed, Fault::Panic, Fault::Disconnect] {
        let calls = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let mods = [("faultfirst", Some(fault)), ("faultsecond", None)]
            .into_iter()
            .enumerate()
            .map(|(index, (name, fault))| {
                ModEntry::new(
                    name,
                    Box::new(FaultRegistration(Arc::new(FaultHandler {
                        fault,
                        calls: calls[index].clone(),
                    }))),
                )
            })
            .collect::<Vec<_>>();
        mod_platform::testing::with_mods(mods.into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport =
                Transport::capture(Side::Server, crate::raw_payloads::TEST_RECORD_OVERHEAD)
                    .unwrap();
            transport.accept(&[]).unwrap();
            transport.finish_configuration().unwrap();
            let (sender, _) = PlayerSender::payload_executor_test_sender(transport.clone());
            let endpoint = Endpoint::server(sender);
            let (events, _receiver) = crate::lan::event_channel();
            let queue = || ServerQueue::new(events.clone(), endpoint.clone(), Some(1));
            let deliver = || {
                let Ok(Classified::Raw(deliveries)) =
                    Incoming::classify(&transport, Phase::Play, serverbound(CHANNEL, 1))
                else {
                    panic!("raw fan-out expected");
                };
                for mut incoming in deliveries {
                    // Raw registration selects Main. Exercise the physical network executor
                    // with the same per-owner receivers, whose main path has its own test.
                    incoming.receiver.thread = mod_api::HandlerThread::Network;
                    dispatch_server(incoming, endpoint.clone(), queue());
                }
            };
            deliver();
            assert_eq!(calls[0].load(Ordering::SeqCst), 1);
            if matches!(fault, Fault::Disconnect) {
                assert!(transport.closed(), "successful raw handlers may disconnect");
                assert_eq!(calls[1].load(Ordering::SeqCst), 0);
            } else {
                assert!(
                    !transport.closed(),
                    "raw handler failure must not disconnect"
                );
                assert_eq!(
                    calls[1].load(Ordering::SeqCst),
                    1,
                    "the other mod's copy must run"
                );
                assert!(!transport.connection.as_ref().unwrap().payloads_pending());
                // Only the panicking registration is quarantined; both ordinary errors
                // and panics leave the other receiver and the transport alive.
                deliver();
                assert!(!transport.closed());
                assert_eq!(
                    calls[0].load(Ordering::SeqCst),
                    if matches!(fault, Fault::Panic) { 1 } else { 2 }
                );
                assert_eq!(calls[1].load(Ordering::SeqCst), 2);
            }
        });
    }
}
