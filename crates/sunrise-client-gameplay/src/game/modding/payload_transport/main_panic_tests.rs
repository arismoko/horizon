use mod_api::{
    __private::{HookPanicReport, take_hook_panics},
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadHandleError, PayloadId,
    Registrar, Text,
};
use std::sync::{Arc, Mutex};
use sunrise_protocol::{
    packets::{configuration::CustomPayload, play},
    payload_negotiation::Phase,
};
use sunrise_sim::{
    lan::PlayerSender,
    raw_payloads::{Endpoint, Incoming, Side, Transport},
};

const GOOD: &str = "payload_panic_probe:good";
const BAD: &str = "payload_panic_probe:bad";

struct Receiver(&'static str, Arc<Mutex<Vec<u8>>>);
impl IPayloadHandler for Receiver {
    fn handle(
        &self,
        bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        self.1.lock().unwrap().push(bytes.0[0]);
        cx.reply(&PayloadId(self.0.into()), bytes)
            .map_err(|err| PayloadHandleError::Failed(format!("could not stage reply: {err:?}")))?;
        if self.0 == BAD {
            cx.disconnect(Text::literal("discard this disconnect"));
            panic!("main payload panic after staging a reply");
        }
        Ok(())
    }
}

struct Registers(Arc<Mutex<Vec<u8>>>);
#[mod_api::__subscriptions]
impl Hooks for Registers {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut handlers = registrar.payload_handlers();
        let mut play = handlers.registrar("1");
        for id in [GOOD, BAD] {
            let callback = Arc::new(Receiver(id, self.0.clone()));
            play.play_bidirectional(PayloadId(id.into()), callback.clone(), Some(callback));
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_panicking_main_payload_refunds_its_journal_and_preserves_unrelated_delivery() {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};

    super::tests::require_resources();
    const NAME: &str = "game::payload_transport::main_panic_tests::a_panicking_main_payload_refunds_its_journal_and_preserves_unrelated_delivery";
    if run_fresh_cases(NAME, "SUNRISE_MAIN_PAYLOAD_PANIC_CHILD", &["main"]) {
        return;
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "payload_panic_probe",
        Box::new(Registers(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay_common::game::api_values::install_parts(mod_api::__private::mods())
            .unwrap();
        with_game(|game| {
            let transport = Transport::capture(
                Side::Server,
                sunrise_sim::raw_payloads::connection::record_overhead(&[]),
            )
            .unwrap();
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            let (sender, sent) = PlayerSender::payload_executor_test_sender(transport.clone());
            let endpoint = Endpoint::server(sender);
            let payload = |channel: &str, marker| CustomPayload {
                channel: channel.into(),
                data: vec![marker],
            };
            // Admit the repeat before the panic to exercise the guard on already-queued work.
            let deliveries = [(GOOD, 1), (BAD, 2), (BAD, 3), (GOOD, 4)].map(|(id, step)| {
                Incoming::new(&transport, Phase::Play, payload(id, step)).unwrap()
            });
            for delivery in deliveries {
                game.run_payload(endpoint.clone(), delivery);
                assert!(!transport.closed());
            }
            assert_eq!(*seen.lock().unwrap(), [1, 2, 4]);
            assert_eq!(
                sent(),
                [1, 4]
                    .map(|step| { play::Clientbound::CustomPayload(payload(GOOD, step)).encode() })
            );
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "payload_panic_probe",
                    hook: "payload",
                }]
            );
        });
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;
