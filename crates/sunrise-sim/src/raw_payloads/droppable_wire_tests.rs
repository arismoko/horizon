//! The group send, the roster, the incoming budget and the drop counts, through a real
//! registration and a real connection table.
use super::{Classified, Incoming, Side, Transport, droppable, encode};
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, PayloadSendError,
    Registrar,
};
use std::sync::Arc;
use sunrise_protocol::{packets::configuration::CustomPayload, payload_negotiation::Phase};

const FRAME: &str = "voice_probe:frame";
const MIC: &str = "voice_probe:mic";
const STATE: &str = "voice_probe:state";

struct Probe;
impl IPayloadHandler for Probe {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut event = registrar.payload_handlers();
        let mut plain = event.registrar("1");
        plain.play_to_client(PayloadId(STATE.into()), None);
        let mut view = plain.droppable();
        view.play_to_client(PayloadId(FRAME.into()), None);
        view.play_to_server(PayloadId(MIC.into()), Arc::new(Probe));
    }
}

fn with_server<R>(run: impl FnOnce(Arc<Transport>) -> R) -> R {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("voice_probe", Box::new(Probe))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport =
                Transport::capture(Side::Server, crate::raw_payloads::TEST_RECORD_OVERHEAD)
                    .unwrap();
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            run(transport)
        },
    )
}
fn player(id: u32, uuid: u128) -> mod_api::PlayerRef {
    mod_api::__private::game::player_ref(
        mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), id),
        uuid,
    )
}

#[test]
fn send_to_queues_on_each_bound_connection_and_a_closed_one_leaves_the_roster() {
    with_server(|transport| {
        let local = &transport.local;
        let (bound, stranger) = (player(1, 11), player(2, 22));
        transport.bind_player(bound);
        let id = PayloadId(FRAME.into());
        let queued = local
            .send_to(&[bound, stranger, bound], &id, PayloadBytes(vec![7, 8]))
            .unwrap();
        // The stranger is skipped; a player listed twice is queued twice.
        assert_eq!(queued, 2);
        let queue = transport
            .connection
            .as_ref()
            .unwrap()
            .droppable()
            .unwrap()
            .clone();
        let expected = encode(Phase::Play, Side::Server, &id, PayloadBytes(vec![7, 8]));
        let now = std::time::Instant::now();
        assert_eq!(queue.pop(now), Some(expected.clone()));
        assert_eq!(queue.pop(now), Some(expected));
        assert_eq!(queue.pop(now), None);
        // Another delivery or an unknown channel refuses; nothing is queued.
        assert_eq!(
            local.send_to(&[bound], &PayloadId(STATE.into()), PayloadBytes(vec![])),
            Err(PayloadSendError::NotDroppable(PayloadId(STATE.into())))
        );
        assert!(matches!(
            local.send_to(
                &[bound],
                &PayloadId("nope:nope".into()),
                PayloadBytes(vec![])
            ),
            Err(PayloadSendError::UnknownPayload(_))
        ));
        assert_eq!(
            local.send_to(&[bound], &PayloadId(MIC.into()), PayloadBytes(vec![])),
            Err(PayloadSendError::WrongDirection(PayloadId(MIC.into())))
        );
        // A send still waiting at close is counted as dropped, and the player leaves the roster.
        local.send_to(&[bound], &id, PayloadBytes(vec![1])).unwrap();
        transport.close();
        let counts = mod_api::__private::game::payload_drop_counts(&id).unwrap();
        assert_eq!(counts.switch, 1);
        assert_eq!(local.send_to(&[bound], &id, PayloadBytes(vec![1])), Ok(0));
    });
}

#[test]
fn a_client_over_the_serverbound_budget_loses_the_send_and_stays() {
    with_server(|transport| {
        let send = |len: usize| {
            Incoming::classify(
                &transport,
                Phase::Play,
                CustomPayload {
                    channel: MIC.into(),
                    data: vec![0; len],
                },
            )
            .unwrap()
        };
        // Within the second's burst, delivered; past it, dropped and counted, no failure.
        let half = droppable::SERVERBOUND_BUDGET as usize * 6 / 10;
        assert!(matches!(send(half), Classified::Application(_)));
        match send(half) {
            Classified::Raw(deliveries) => assert!(deliveries.is_empty()),
            _ => panic!("the second send must be dropped"),
        }
        let counts = mod_api::__private::game::payload_drop_counts(&PayloadId(MIC.into())).unwrap();
        assert_eq!(counts.budget, 1);
    });
}
