//! A dedicated payload receiver parks, and an epoch end wakes it while the main
//! root still holds its turn.
use crate::lan::{NetEvent, PlayerSender};
use crate::raw_payloads::vanilla_peer_support::{CHANNEL, serverbound, with_vanilla_peer};
use crate::raw_payloads::{
    Classified, Endpoint, Incoming, ServerQueue, connection::test_hooks, dispatch_server,
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use sunrise_protocol::payload_negotiation::Phase;

#[test]
fn epoch_end_wakes_a_dedicated_payload_receiver_before_the_main_root_finishes() {
    mod_platform::__private::initialize().unwrap();

    with_vanilla_peer(|transport| {
        let epoch = transport.epoch().unwrap();
        let Classified::Raw(deliveries) =
            Incoming::classify(&transport, Phase::Play, serverbound(CHANNEL, 1)).unwrap()
        else {
            panic!("expected one delivery per raw-channel listener");
        };
        let [first, later]: [Incoming; 2] = deliveries.try_into().ok().unwrap();
        let first_lease = Arc::downgrade(&first.lease);
        let later_lease = Arc::downgrade(&later.lease);
        let (sender, _) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let (events, receiver) = crate::lan::event_channel();

        dispatch_server(
            first,
            endpoint.clone(),
            ServerQueue::new(events.clone(), endpoint.clone(), Some(1)),
        );
        let mut turn = receiver.take_turn().into_iter();
        let (NetEvent::RawPayload(held), _credit) = turn.next().unwrap().into_parts() else {
            panic!("expected the main root");
        };
        assert!(turn.next().is_none());

        let (entered, parked) = mpsc::channel();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            test_hooks::arm(entered);
            dispatch_server(
                later,
                endpoint.clone(),
                ServerQueue::new(events, endpoint, None),
            );
            let _ = done.send(());
        });
        let bound = Duration::from_secs(2);
        parked
            .recv_timeout(bound)
            .expect("the dedicated waiter parks");
        transport.begin_configuration();
        result
            .recv_timeout(bound)
            .expect("epoch invalidation wakes the receiver while the main root is held");
        assert!(!epoch.active());
        assert!(first_lease.upgrade().is_some());
        assert!(later_lease.upgrade().is_none());
        assert!(receiver.take_turn().is_empty(), "old work was canceled");
        drop(held);
        worker.join().unwrap();
    });
}
