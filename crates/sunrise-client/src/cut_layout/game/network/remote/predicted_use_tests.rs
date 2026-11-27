//! A native listener rewriting the real client prediction hand loop.
use super::*;
#[path = "predicted_use_tests/fixture.rs"]
pub(crate) mod fixture;

pub(crate) struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(registrar);
    }
    fn entity_interact(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<Answer> {
        fixture::entity_interact(host, event)
    }
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        fixture::packet(event);
        Flow::Continue
    }
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::remote_server::interaction_event_tests::predicted_use_tests::matrix;

#[test]
#[ignore = "daily"]
fn native_client_prediction_owns_the_hand_loop() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::interaction_event_tests::predicted_use_tests::native_client_prediction_owns_the_hand_loop",
        || {
            matrix(
                vec![mod_api::ModEntry::new(
                    "prediction_fixture",
                    Box::new(Native),
                )]
                .into(),
            )
        },
    );
}
