#![cfg(feature = "client")]
use mod_api::{
    BlockUpdateFlags, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadHandleError, PayloadId, PayloadWork, Registrar, TransactionError,
    prelude::BlockStatesHost,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    packets::{configuration as cfg, play},
    payload_negotiation::Phase,
};
use sunrise_sim::{
    lan::PlayerSender,
    raw_payloads::{Classified, Endpoint, Incoming, Side, Transport},
};

const CHANNEL: &str = "work_probe:message";
const AT: [i32; 3] = [1, 64, 1];

#[derive(Default)]
struct Handler {
    pub staged: Arc<AtomicBool>,
    calls: AtomicUsize,
}
struct Work(Arc<AtomicBool>);

impl PayloadWork for Work {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        let host = cx.server_host().unwrap();
        let level = host.level("minecraft:overworld").unwrap();
        let stone = host.block_state_id("minecraft:stone").unwrap();
        assert_eq!(
            level.set_block(host, AT, stone, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(level.block_state(host, AT), Some(stone));
        cx.reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![1]))
            .unwrap();
        self.0.store(true, Ordering::Relaxed);
        panic!("work panicked after staging a block change and a reply");
    }
}

impl IPayloadHandler for Handler {
    fn handle(
        &self,
        _: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let child = cx.transaction(|child, _| {
            child
                .enqueue_work(Arc::new(Work(self.staged.clone())))
                .map(drop)
        });
        assert_eq!(
            child,
            Err(TransactionError::Aborted(PayloadContextError::Disconnected))
        );
        cx.reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![2]))
            .unwrap();
        Ok(())
    }
}

struct Declares(Arc<Handler>);
#[mod_api::__subscriptions]
impl Hooks for Declares {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .play_raw(PayloadId(CHANNEL.into()), Some(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn a_handled_child_abort_rolls_back_work_but_still_reports_the_panicking_owner() {
    use {
        sunrise_client_gameplay::game::toast_manager::Toast,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    if run_fresh_cases(
        "cut_layout::game::payload_transport::work_panic_tests::a_handled_child_abort_rolls_back_work_but_still_reports_the_panicking_owner",
        "SUNRISE_PAYLOAD_WORK_PANIC_CHILD",
        &["run"],
    ) {
        return;
    }
    let handler = Arc::new(Handler::default());
    let mods = vec![ModEntry::new(
        "work_probe",
        Box::new(Declares(handler.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let air = BlockStatesHost::block_state_id(game, "minecraft:air").unwrap();
            assert_eq!(level.block_state(game, AT), Some(air));

            let transport = Transport::capture(
                Side::Server,
                sunrise_sim::raw_payloads::connection::record_overhead(&[]),
            )
            .unwrap();
            transport.accept(&[]).unwrap();
            transport.finish_configuration().unwrap();
            let (sender, sent) = PlayerSender::payload_executor_test_sender(transport.clone());
            let endpoint = Endpoint::server(sender);
            let packet = || cfg::CustomPayload {
                channel: CHANNEL.into(),
                data: vec![],
            };
            let incoming = || Incoming::new(&transport, Phase::Play, packet()).unwrap();
            let message = incoming();
            // Admit a repeat before the panic to exercise the guard on already-queued work.
            let queued_repeat = incoming();
            let receiver = message.receiver.clone();
            assert!(receiver.enabled());
            game.run_payload(endpoint.clone(), message);
            assert!(handler.staged.load(Ordering::Relaxed));
            assert_eq!(level.block_state(game, AT), Some(air));
            // The parent committed its reply, but the aborted child sent nothing.
            assert_eq!(
                sent(),
                [play::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: CHANNEL.into(),
                    data: vec![2],
                })
                .encode()]
            );
            assert!(!receiver.enabled());
            game.run_payload(endpoint, queued_repeat);
            assert_eq!(handler.calls.load(Ordering::Relaxed), 1);
            // Later packets for the poisoned raw receiver are skipped before admission.
            assert!(matches!(
                Incoming::classify(&transport, Phase::Play, packet()),
                Ok(Classified::Raw(deliveries)) if deliveries.is_empty()
            ));

            // The client drains the shared queue, not the server action that aborted.
            crate::game::server_side::native_entity_refs::tests::with_client_game(|client| {
                client.test_add_hook_panic_toasts();
                assert_eq!(client.client.toast_manager.queued.len(), 1);
                let Some(Toast::Diagnostic { toast, .. }) =
                    client.client.toast_manager.queued.front()
                else {
                    panic!("expected exactly one shared panic toast");
                };
                assert_eq!(
                    toast.message.as_ref().unwrap().component(),
                    serde_json::json!({
                        "translate": "sunrise.mods.hook_panic.description",
                        "with": [{"text": "work_probe"}, {"text": "payload work"}],
                    })
                );
            });
        });
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;
