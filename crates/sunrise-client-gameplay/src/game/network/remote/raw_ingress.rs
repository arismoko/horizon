//! Receiving-provider credit follows ordered events and decoder placeholders.
#[cfg(any(test, feature = "test-support"))]
use super::Received;
use super::opening::RemoteOpening;
#[cfg(any(test, feature = "test-support"))]
use std::time::{Duration, Instant};
use sunrise_client_lib::net::Event;

pub(in crate::game) fn local_transfer_notice() -> Event {
    Event::Disconnected {
        reason: sunrise_game_data::lang::get("disconnect.transfer"),
        key: Some("disconnect.transfer".into()),
        from_server: true,
        after_login: true,
    }
}

pub(in crate::game) fn packet_error_notice() -> Event {
    Event::Disconnected {
        reason: sunrise_game_data::lang::get("disconnect.packetError"),
        key: Some("disconnect.packetError".into()),
        from_server: false,
        after_login: true,
    }
}

#[derive(Default)]
pub struct ListenerTurn {
    pub(in crate::game) opening: Option<RemoteOpening>,
    pub end: Option<Event>,
    pub(in crate::game) packet_error: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    pub fn wait_for_test_remote_event(&mut self, until: impl Fn(&Event) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let remote = self.client.remote.as_mut().unwrap();
        let mut backlog = std::mem::take(&mut remote.backlog);
        loop {
            remote.receive_credited_backlog(&mut backlog);
            if backlog
                .iter()
                .any(|item| matches!(item, Received::Event(event, _) if until(event)))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected event did not reach main"
            );
            std::thread::yield_now();
        }
        remote.backlog = backlog;
    }
}

#[cfg(feature = "client")]
#[cfg(any(test, feature = "test-support"))]
impl crate::game::ClientGame<'_> {
    pub fn acknowledge_test_configuration(&mut self) {
        self.wait_for_test_remote_event(|event| matches!(event, Event::FinishConfiguration(_)));
        self.poll_remote_events(true).unwrap();
    }

    pub fn drain_test_remote_join(&mut self) {
        self.wait_for_test_remote_event(|event| matches!(event, Event::Joined { .. }));
        self.poll_remote_events(true).unwrap();
    }

    pub fn dispatch_test_remote_event(&mut self, event: Event) -> super::Result<()> {
        let (tx, rx) = sunrise_client_lib::net::ingress::channel(1024, |_| 1).unwrap();
        tx.send(event).unwrap();
        let (event, credit) = rx.try_recv_owned().unwrap().into_parts();
        self.client
            .remote
            .as_mut()
            .unwrap()
            .backlog
            .push_back(Received::Event(event, credit));
        self.poll_remote_events(true)
    }
}

/// Protocol error raised while handling a play packet (`ClientCommonPacketListenerImpl.onPacketError`).
#[derive(Debug)]
pub struct PacketError(pub(in crate::game) &'static str);

impl std::fmt::Display for PacketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for PacketError {}
