//! `ServerCommonPacketListenerImpl.keepConnectionAlive`: a guest that answers no keep-alive is
//! timed out, except the integrated server's own player (`!isSingleplayerOwner()`).
use super::pending_move_tests::with_input_context;
use super::*;

/// A hub whose one guest, "Bob", has answered no keep-alive for longer than the timeout, on a
/// server whose singleplayer profile is `owner`'s.
fn hub_with_a_silent_guest(owner: Option<&str>) -> Hub {
    let (mut hub, _rx) = hub_with_net(
        SyncData::empty(),
        crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: owner.map(|name| (name.to_owned(), 1)),
            ..crate::lan::NetConfig::loopback(0, "KeepAliveOwner".into(), 2)
        },
    );
    hub.guests.update(0, |g| {
        g.sender = crate::lan::PlayerSender::timed_out_test_sender();
    });
    hub
}

/// The timeout is judged once a second (every 20th tick).
fn judge_timeouts(hub: &mut Hub) {
    hub.ticks = 19;
    with_input_context(|ctx| drop(hub.poll(ctx)));
}

#[test]
fn the_singleplayer_owner_is_never_timed_out() {
    // The owner's name is matched ignoring case, as `IntegratedServer.isSingleplayerOwner` does.
    let mut hub = hub_with_a_silent_guest(Some("bob"));
    judge_timeouts(&mut hub);
    assert!(!hub.guests[0].disconnect_pending);
    assert!(hub.guests[0].left_reason.is_empty());
}

#[test]
fn any_other_silent_player_is_timed_out() {
    for owner in [None, Some("Alex")] {
        let mut hub = hub_with_a_silent_guest(owner);
        judge_timeouts(&mut hub);
        assert!(hub.guests[0].disconnect_pending, "owner {owner:?}");
    }
}
