//! A vanilla-shaped peer with two raw receivers and a send-only declaration,
//! shared by the raw-channel tests and the connection tests.
use super::{Side, Transport};
use mod_api::{
    Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, Registrar,
};
use std::sync::Arc;
use sunrise_protocol::packets::custom_payload::CustomPayload;

pub(crate) const CHANNEL: &str = "voicechat:secret";
pub(crate) const SEND_ONLY: &str = "bungeecord:main";

struct Listener;
impl IPayloadHandler for Listener {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
pub(crate) struct Declares {
    pub(crate) receives: bool,
    pub(crate) send_only: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Declares {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut payloads = registrar.payload_handlers();
        let mut raw = payloads.registrar("1");
        if self.receives {
            raw.play_raw(PayloadId(CHANNEL.into()), Some(Arc::new(Listener)));
        }
        if self.send_only {
            raw.play_raw(PayloadId(SEND_ONLY.into()), None);
        }
    }
}
pub(crate) struct Typed;
impl IPayloadHandler for Typed {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        Ok(())
    }
}
#[mod_api::__subscriptions]
impl Hooks for Typed {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_to_server(PayloadId(CHANNEL.into()), Arc::new(Typed));
    }
}
/// Two receiving mods and a send-only one, with a vanilla peer admitted
/// without an offer, as `finish_without_offer` admits it.
pub(crate) fn with_vanilla_peer(run: impl FnOnce(Arc<Transport>)) {
    let mods = vec![
        ModEntry::new(
            "voicechat",
            Box::new(Declares {
                receives: true,
                send_only: false,
            }),
        ),
        ModEntry::new(
            "voicebridge",
            Box::new(Declares {
                receives: true,
                send_only: true,
            }),
        ),
    ];
    mod_platform::testing::with_mods(mods.into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
            .unwrap();
        let transport =
            Transport::capture(Side::Server, crate::raw_payloads::TEST_RECORD_OVERHEAD).unwrap();
        assert!(
            !transport.local.has_mod_declarations(),
            "raw channels are never offered"
        );
        assert!(transport.connection.is_some(), "raw-only still keeps state");
        transport.accept(&[]).unwrap();
        transport.finish_configuration().unwrap();
        run(transport)
    });
}

pub(crate) fn serverbound(channel: &str, len: usize) -> CustomPayload {
    CustomPayload {
        channel: channel.into(),
        data: vec![7; len],
    }
}
